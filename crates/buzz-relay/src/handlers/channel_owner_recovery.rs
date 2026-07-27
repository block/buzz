//! Dedicated orphaned-channel ownership recovery handler (kind 9038).

use std::sync::Arc;

use nostr::{Event, EventBuilder, Kind, Tag};
use uuid::Uuid;

use buzz_core::kind::{CHANNEL_OWNER_RECOVERY_AUDIT_MARKER, KIND_SYSTEM_MESSAGE};
use buzz_core::tenant::TenantContext;

use crate::handlers::event::dispatch_persistent_event;
use crate::handlers::side_effects::emit_group_discovery_events;
use crate::state::AppState;

/// Validate, atomically apply, and deliver a protected recovery request.
pub async fn handle_channel_owner_recovery(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<String, String> {
    let (channel_id, target_hex, reason) = parse_request(event)?;
    let target = hex::decode(&target_hex).map_err(|_| "invalid target pubkey".to_string())?;

    let record = state
        .db
        .recover_channel_owner(tenant.community(), channel_id, &target, &reason, event)
        .await
        .map_err(|e| e.to_string())?;

    // Repeat both cache invalidation and discovery publication on idempotent
    // request replay. That gives a committed recovery a convergence path after
    // a transient post-commit failure without repeating the promotion.
    state.invalidate_membership(tenant, channel_id, &target);
    if let Err(error) = emit_group_discovery_events(tenant, state, channel_id).await {
        tracing::warn!(
            channel = %channel_id,
            request = %event.id,
            error = %error,
            "owner recovery committed but group discovery refresh failed"
        );
    }

    if !record.delivered {
        if let Err(error) = deliver_audit_event(
            tenant,
            state,
            event.id.as_bytes().as_slice(),
            &record.payload,
        )
        .await
        {
            let error_text = error.to_string();
            tracing::warn!(
                channel = %channel_id,
                request = %event.id,
                error = %error_text,
                "owner recovery committed; audit event remains in durable outbox"
            );
            if let Err(db_error) = state
                .db
                .record_recovery_delivery_failure(
                    tenant.community(),
                    event.id.as_bytes().as_slice(),
                    &error_text,
                )
                .await
            {
                tracing::error!(
                    request = %event.id,
                    error = %db_error,
                    "failed to record owner recovery audit delivery failure"
                );
            }
            return Ok("recovered; channel audit delivery pending retry".into());
        }
    }

    Ok(if record.applied {
        "channel ownership recovered".into()
    } else {
        "channel ownership recovery already applied".into()
    })
}

async fn deliver_audit_event(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    request_event_id: &[u8],
    payload: &buzz_db::channel_owner_recovery::RecoveryAuditPayload,
) -> anyhow::Result<()> {
    let audit = build_audit_event(&state.relay_keypair, payload)?;
    let (stored, inserted) = state
        .db
        .store_recovery_audit_event(
            tenant.community(),
            request_event_id,
            &audit,
            payload.channel_id,
        )
        .await?;
    if inserted {
        let relay_pubkey = state.relay_keypair.public_key().to_hex();
        dispatch_persistent_event(
            tenant,
            state,
            &stored,
            KIND_SYSTEM_MESSAGE,
            &relay_pubkey,
            None,
        )
        .await;
    }
    Ok(())
}

/// Drain a bounded batch from the durable recovery-audit outbox.
///
/// The relay-signed audit event is deterministic, so concurrent relay pods
/// converge on one persisted event ID before each marks the outbox row
/// delivered.
pub async fn drain_pending_recovery_audits(
    state: &Arc<AppState>,
    limit: i64,
) -> anyhow::Result<usize> {
    let pending = state.db.pending_recovery_deliveries(limit).await?;
    let mut delivered = 0;
    for item in pending {
        let tenant = TenantContext::resolved(item.community_id, item.host);
        match deliver_audit_event(&tenant, state, &item.request_event_id, &item.payload).await {
            Ok(()) => delivered += 1,
            Err(error) => {
                let error_text = error.to_string();
                tracing::warn!(
                    community = %item.community_id,
                    channel = %item.payload.channel_id,
                    request = %hex::encode(&item.request_event_id),
                    error = %error_text,
                    "pending owner recovery audit delivery failed"
                );
                if let Err(db_error) = state
                    .db
                    .record_recovery_delivery_failure(
                        item.community_id,
                        &item.request_event_id,
                        &error_text,
                    )
                    .await
                {
                    tracing::error!(
                        request = %hex::encode(&item.request_event_id),
                        error = %db_error,
                        "failed to record pending recovery audit delivery failure"
                    );
                }
            }
        }
    }
    Ok(delivered)
}

fn build_audit_event(
    relay_keys: &nostr::Keys,
    payload: &buzz_db::channel_owner_recovery::RecoveryAuditPayload,
) -> anyhow::Result<Event> {
    let content = serde_json::to_string(payload)?;
    let channel = payload.channel_id.to_string();
    let created_at = payload.created_at.timestamp().max(0) as u64;
    Ok(
        EventBuilder::new(Kind::Custom(KIND_SYSTEM_MESSAGE as u16), content)
            .tags([
                Tag::parse(["h", &channel])?,
                Tag::parse(["e", &payload.request_event_id])?,
                Tag::parse(["p", &payload.actor])?,
                Tag::parse(["p", &payload.target])?,
                Tag::parse(["predicate", &payload.predicate_id])?,
                Tag::parse(["reason-code", &payload.reason_code])?,
                Tag::parse(["audit", CHANNEL_OWNER_RECOVERY_AUDIT_MARKER])?,
            ])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(relay_keys)?,
    )
}

fn parse_request(event: &Event) -> Result<(Uuid, String, String), String> {
    if !event.content.is_empty() {
        return Err("recovery request content must be empty".into());
    }
    let mut protected = 0;
    let mut channel = None;
    let mut target = None;
    let mut reason = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(String::as_str) {
            Some("-") if parts.len() == 1 => protected += 1,
            Some("h") if parts.len() == 2 && channel.is_none() => {
                channel = Some(
                    Uuid::parse_str(&parts[1]).map_err(|_| "invalid channel h tag".to_string())?,
                );
            }
            Some("p") if parts.len() == 2 && target.is_none() => {
                let value = &parts[1];
                if value.len() != 64
                    || !value.chars().all(|ch| ch.is_ascii_hexdigit())
                    || value.to_ascii_lowercase() != *value
                {
                    return Err("target p tag must be lowercase 64-character hex".into());
                }
                target = Some(value.clone());
            }
            Some("reason") if parts.len() == 2 && reason.is_none() => {
                let value = parts[1].trim();
                if value.is_empty() || value.len() > 500 || value.chars().any(char::is_control) {
                    return Err("reason must be 1–500 bytes without control characters".into());
                }
                reason = Some(value.to_string());
            }
            _ => return Err("malformed, duplicate, or unsupported recovery request tag".into()),
        }
    }
    if protected != 1 {
        return Err("request must include exactly one NIP-70 protected tag".into());
    }
    let channel = channel.ok_or_else(|| "missing channel h tag".to_string())?;
    if channel.is_nil() {
        return Err("channel h tag must not be nil".into());
    }
    Ok((
        channel,
        target.ok_or_else(|| "missing target p tag".to_string())?,
        reason.ok_or_else(|| "missing reason tag".to_string())?,
    ))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone as _, Utc};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use buzz_core::kind::KIND_CHANNEL_OWNER_RECOVERY;

    use super::*;

    fn request(tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(KIND_CHANNEL_OWNER_RECOVERY as u16), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn request_parser_requires_exact_protected_shape() {
        let channel = Uuid::new_v4();
        let target = "11".repeat(32);
        let event = request(
            vec![
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(["p", &target]).unwrap(),
                Tag::parse(["reason", "durable self-consent recorded"]).unwrap(),
            ],
            "",
        );
        assert_eq!(
            parse_request(&event).unwrap(),
            (channel, target, "durable self-consent recorded".to_string())
        );
    }

    #[test]
    fn request_parser_rejects_duplicates_unknown_tags_and_content() {
        let channel = Uuid::new_v4().to_string();
        let target = "11".repeat(32);
        for tags in [
            vec![
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &channel]).unwrap(),
                Tag::parse(["p", &target]).unwrap(),
                Tag::parse(["reason", "reason"]).unwrap(),
            ],
            vec![
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &channel]).unwrap(),
                Tag::parse(["p", &target]).unwrap(),
                Tag::parse(["reason", "reason"]).unwrap(),
                Tag::parse(["x", "unexpected"]).unwrap(),
            ],
        ] {
            assert!(parse_request(&request(tags, "")).is_err());
        }
        let valid_tags = vec![
            Tag::parse(["-"]).unwrap(),
            Tag::parse(["h", &channel]).unwrap(),
            Tag::parse(["p", &target]).unwrap(),
            Tag::parse(["reason", "reason"]).unwrap(),
        ];
        assert!(parse_request(&request(valid_tags, "not empty")).is_err());
    }

    #[test]
    fn request_parser_rejects_missing_and_malformed_required_tags() {
        let channel = Uuid::new_v4().to_string();
        let target = "11".repeat(32);
        let valid = || {
            vec![
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &channel]).unwrap(),
                Tag::parse(["p", &target]).unwrap(),
                Tag::parse(["reason", "reason"]).unwrap(),
            ]
        };

        for missing_index in 0..4 {
            let mut tags = valid();
            tags.remove(missing_index);
            assert!(parse_request(&request(tags, "")).is_err());
        }

        for replacement in [
            Tag::parse(["h", "not-a-uuid"]).unwrap(),
            Tag::parse(["h", &Uuid::nil().to_string()]).unwrap(),
            Tag::parse(["p", &"1".repeat(63)]).unwrap(),
            Tag::parse(["p", &"AA".repeat(32)]).unwrap(),
            Tag::parse(["reason", "bad\nreason"]).unwrap(),
        ] {
            let mut tags = valid();
            let tag_name = replacement.as_slice()[0].clone();
            let index = tags
                .iter()
                .position(|tag| tag.as_slice()[0] == tag_name)
                .expect("replace required tag");
            tags[index] = replacement;
            assert!(parse_request(&request(tags, "")).is_err());
        }
    }

    #[test]
    fn audit_event_is_exact_relay_signed_and_deterministic() {
        let relay = Keys::generate();
        let payload = buzz_db::channel_owner_recovery::RecoveryAuditPayload {
            schema_version: 1,
            event_type: "channel_owner_recovered".into(),
            community_id: Uuid::new_v4(),
            channel_id: Uuid::new_v4(),
            request_event_id: "22".repeat(32),
            actor: "33".repeat(32),
            target: "44".repeat(32),
            predicate_id: buzz_db::channel_owner_recovery::RECOVERY_PREDICATE_ID.into(),
            reason_code: buzz_db::channel_owner_recovery::RECOVERY_REASON_CODE.into(),
            reason: "durable consent recovery".into(),
            prior_elevated_roles: vec![buzz_db::channel_owner_recovery::PriorElevatedRole {
                pubkey: "55".repeat(32),
                role: "owner".into(),
            }],
            created_at: Utc.timestamp_opt(1_750_000_000, 0).unwrap(),
        };

        let first = build_audit_event(&relay, &payload).expect("build audit event");
        let replay = build_audit_event(&relay, &payload).expect("rebuild audit event");
        assert_eq!(first.id, replay.id);
        assert_eq!(first.pubkey, relay.public_key());
        assert_eq!(first.kind, Kind::Custom(KIND_SYSTEM_MESSAGE as u16));
        assert_eq!(
            serde_json::from_str::<buzz_db::channel_owner_recovery::RecoveryAuditPayload>(
                &first.content
            )
            .expect("decode payload"),
            payload
        );
        let tags: Vec<Vec<String>> = first
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert_eq!(
            tags,
            vec![
                vec!["h".into(), payload.channel_id.to_string()],
                vec!["e".into(), payload.request_event_id],
                vec!["p".into(), payload.actor],
                vec!["p".into(), payload.target],
                vec!["predicate".into(), payload.predicate_id],
                vec!["reason-code".into(), payload.reason_code],
                vec!["audit".into(), CHANNEL_OWNER_RECOVERY_AUDIT_MARKER.into(),],
            ]
        );
        buzz_core::verification::verify_event(&first).expect("verify relay signature");
    }

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        sqlx::PgPool::connect(&url)
            .await
            .expect("connect to recovery integration test database")
    }

    async fn test_state(pool: sqlx::PgPool) -> Arc<AppState> {
        let db = buzz_db::Db::from_pool(pool.clone());
        let config = crate::config::Config::from_env().expect("load relay test configuration");
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("create recovery integration Redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("connect recovery integration pubsub"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage =
            buzz_media::MediaStorage::new(&config.media).expect("create test media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres and Redis"]
    async fn successful_handler_persists_and_delivers_channel_visible_audit() {
        let pool = test_pool().await;
        sqlx::query("SELECT 1 FROM channel_owner_recovery_audit LIMIT 1")
            .execute(&pool)
            .await
            .expect("owner recovery migration must be applied");
        let state = test_state(pool.clone()).await;

        let community_uuid = Uuid::new_v4();
        let host = format!("owner-recovery-handler-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert community");
        let community = buzz_core::CommunityId::from_uuid(community_uuid);
        let tenant = TenantContext::resolved(community, host);
        let actor = Keys::generate();
        let owner = Keys::generate();
        let target = Keys::generate();
        for keys in [&actor, &owner, &target] {
            sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2)")
                .bind(community.as_uuid())
                .bind(keys.public_key().to_bytes().as_slice())
                .execute(&pool)
                .await
                .expect("insert human");
        }
        for (keys, role) in [(&actor, "owner"), (&owner, "member"), (&target, "member")] {
            sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,$3)")
                .bind(community.as_uuid())
                .bind(keys.public_key().to_hex())
                .bind(role)
                .execute(&pool)
                .await
                .expect("insert community member");
        }
        let channel = buzz_db::channel::create_channel(
            &pool,
            community,
            "recoverable",
            buzz_core::channel::ChannelType::Stream,
            buzz_core::channel::ChannelVisibility::Open,
            None,
            owner.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel")
        .id;
        buzz_db::channel::add_member(
            &pool,
            community,
            channel,
            target.public_key().to_bytes().as_slice(),
            buzz_core::channel::MemberRole::Member,
            Some(owner.public_key().to_bytes().as_slice()),
        )
        .await
        .expect("add target");
        state
            .db
            .archive(
                community,
                &owner.public_key().to_hex(),
                "self",
                &owner.public_key().to_hex(),
                Some("retired"),
                Some(&target.public_key().to_hex()),
                &"ab".repeat(32),
            )
            .await
            .expect("archive owner");

        let channel_tag = channel.to_string();
        let target_tag = target.public_key().to_hex();
        let request = EventBuilder::new(Kind::Custom(KIND_CHANNEL_OWNER_RECOVERY as u16), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["h", &channel_tag]).unwrap(),
                Tag::parse(["p", &target_tag]).unwrap(),
                Tag::parse(["reason", "handler delivery test"]).unwrap(),
            ])
            .sign_with_keys(&actor)
            .expect("sign request");
        let message = handle_channel_owner_recovery(&tenant, &state, &request)
            .await
            .expect("handle recovery");
        assert_eq!(message, "channel ownership recovered");

        let (delivered, audit_event_id): (Option<DateTime<Utc>>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT delivered_at, audit_event_id FROM channel_owner_recovery_outbox \
             WHERE community_id=$1 AND request_event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(request.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .expect("read outbox");
        assert!(delivered.is_some());

        let events = state
            .db
            .query_events(&buzz_db::EventQuery {
                channel_id: Some(channel),
                kinds: Some(vec![KIND_SYSTEM_MESSAGE as i32]),
                limit: Some(10),
                ..buzz_db::EventQuery::for_community(community)
            })
            .await
            .expect("query channel audit");
        let audit = events
            .into_iter()
            .find(|stored| stored.event.content.contains("channel_owner_recovered"))
            .expect("channel-visible recovery audit");
        let payload: buzz_db::channel_owner_recovery::RecoveryAuditPayload =
            serde_json::from_str(&audit.event.content).expect("decode audit");
        assert_eq!(payload.actor, actor.public_key().to_hex());
        assert_eq!(payload.target, target.public_key().to_hex());
        assert_eq!(payload.reason, "handler delivery test");
        buzz_core::verification::verify_event(&audit.event).expect("verify relay signature");
        assert_eq!(
            audit_event_id.as_deref(),
            Some(audit.event.id.as_bytes().as_slice())
        );
        assert!(state
            .db
            .is_recovery_audit_event(community, audit.event.id.as_bytes().as_slice())
            .await
            .expect("read durable audit link"));
    }
}
