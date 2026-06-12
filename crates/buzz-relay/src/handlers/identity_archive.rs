//! NIP-IA identity archive request handler (kinds 9035–9036).
//!
//! These events are processed before storage: the request mutates the
//! `archived_identities` table and may emit relay-signed NIP-IA deltas and a
//! snapshot, then the ingest pipeline stores the request itself for audit.

use std::sync::Arc;

use nostr::{Event, PublicKey};
use tracing::{info, warn};

use buzz_core::kind::{KIND_IA_ARCHIVE_REQUEST, KIND_IA_UNARCHIVE_REQUEST, KIND_PROFILE};
use buzz_db::EventQuery;

use crate::handlers::side_effects::{
    publish_nipia_archival_list, publish_nipia_archived, publish_nipia_unarchived,
};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsentPath {
    SelfSigned,
    Owner,
    Admin,
}

impl ConsentPath {
    fn as_str(self) -> &'static str {
        match self {
            Self::SelfSigned => "self",
            Self::Owner => "owner",
            Self::Admin => "admin",
        }
    }
}

/// Validate and execute a NIP-IA archive/unarchive request.
pub async fn handle_identity_archive_event(
    state: &Arc<AppState>,
    event: &Event,
) -> Result<(), String> {
    let kind = event.kind.as_u16() as u32;
    let actor_hex = event.pubkey.to_hex();

    if kind != KIND_IA_ARCHIVE_REQUEST && kind != KIND_IA_UNARCHIVE_REQUEST {
        return Err(format!("unexpected identity archive kind: {kind}"));
    }

    enforce_freshness(event)?;
    require_single_protected_tag(event)?;

    let target_hex = extract_single_p_tag_hex(event)
        .ok_or_else(|| "missing or invalid p tag".to_string())?
        .to_ascii_lowercase();

    let replaced_by = extract_optional_replaced_by(event, &target_hex)?;
    if kind == KIND_IA_UNARCHIVE_REQUEST && replaced_by.is_some() {
        return Err("replaced-by is not valid on unarchive requests".to_string());
    }

    let reason = extract_tag_value(event, "reason");
    let consent_path = determine_consent_path(state, event, &target_hex, &actor_hex).await?;
    let request_event_id = event.id.to_hex();

    let changed = if kind == KIND_IA_ARCHIVE_REQUEST {
        state
            .db
            .archive(
                &target_hex,
                consent_path.as_str(),
                &actor_hex,
                reason.as_deref(),
                replaced_by.as_deref(),
                &request_event_id,
            )
            .await
            .map_err(|e| format!("database error: {e}"))?
    } else {
        state
            .db
            .unarchive(&target_hex)
            .await
            .map_err(|e| format!("database error: {e}"))?
    };

    info!(
        actor = %actor_hex,
        target = %target_hex,
        consent = consent_path.as_str(),
        changed,
        kind,
        "identity archive request processed"
    );

    if !changed {
        return Ok(());
    }

    let publish_delta = if kind == KIND_IA_ARCHIVE_REQUEST {
        publish_nipia_archived(
            state,
            &target_hex,
            consent_path.as_str(),
            &actor_hex,
            &request_event_id,
            &event.content,
            reason.as_deref(),
            replaced_by.as_deref(),
        )
        .await
    } else {
        publish_nipia_unarchived(
            state,
            &target_hex,
            consent_path.as_str(),
            &actor_hex,
            &request_event_id,
            &event.content,
            reason.as_deref(),
        )
        .await
    };

    if let Err(e) = publish_delta {
        warn!(error = %e, "failed to publish NIP-IA delta");
    }
    if let Err(e) = publish_nipia_archival_list(state).await {
        warn!(error = %e, "failed to publish NIP-IA archival list");
    }

    Ok(())
}

fn enforce_freshness(event: &Event) -> Result<(), String> {
    let event_ts = event.created_at.as_secs() as i64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if (event_ts - now).abs() > 120 {
        return Err(format!(
            "event timestamp out of range: created_at={event_ts}, now={now}, delta={}s (max ±120s)",
            event_ts - now
        ));
    }
    Ok(())
}

fn require_single_protected_tag(event: &Event) -> Result<(), String> {
    let count = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(|s| s.as_str()) == Some("-"))
        .count();
    if count != 1 {
        return Err(format!(
            "request must include exactly one NIP-70 protected event tag [\"-\"] (got {count})"
        ));
    }
    Ok(())
}

fn extract_single_p_tag_hex(event: &Event) -> Option<String> {
    let mut found = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) != Some("p") {
            continue;
        }
        let val = parts.get(1)?.as_str();
        if val.len() != 64 || !val.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        if found.is_some() {
            return None;
        }
        found = Some(val.to_string());
    }
    found
}

fn extract_tag_value(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) == Some(name) {
            parts.get(1).map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn extract_optional_replaced_by(event: &Event, target_hex: &str) -> Result<Option<String>, String> {
    let mut found = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) != Some("replaced-by") {
            continue;
        }
        let val = parts
            .get(1)
            .ok_or_else(|| "invalid replaced-by tag".to_string())?
            .to_string();
        if val.len() != 64
            || !val.chars().all(|c| c.is_ascii_hexdigit())
            || val.to_ascii_lowercase() != val
        {
            return Err("invalid replaced-by pubkey".to_string());
        }
        if val == target_hex {
            return Err("replaced-by must differ from target".to_string());
        }
        if found.is_some() {
            return Err("multiple replaced-by tags".to_string());
        }
        found = Some(val);
    }
    Ok(found)
}

async fn determine_consent_path(
    state: &Arc<AppState>,
    event: &Event,
    target_hex: &str,
    actor_hex: &str,
) -> Result<ConsentPath, String> {
    if actor_hex == target_hex {
        return Ok(ConsentPath::SelfSigned);
    }

    let actor_member = state
        .db
        .get_relay_member(actor_hex)
        .await
        .map_err(|e| format!("database error: {e}"))?;
    let actor_role = actor_member.as_ref().map(|m| m.role.as_str()).unwrap_or("");
    if actor_role == "owner" || actor_role == "admin" {
        return Ok(ConsentPath::Admin);
    }

    verify_owner_consent(state, event, target_hex, actor_hex).await?;
    Ok(ConsentPath::Owner)
}

async fn verify_owner_consent(
    state: &Arc<AppState>,
    event: &Event,
    target_hex: &str,
    actor_hex: &str,
) -> Result<(), String> {
    let request_auth = extract_single_auth_tag_json(event)?;
    let request_owner = verify_auth_tag_owner(&request_auth, target_hex)
        .map_err(|e| format!("invalid request auth tag: {e}"))?;
    if request_owner != actor_hex {
        return Err("request auth owner must equal request signer".to_string());
    }
    enforce_request_auth_time_bounds(&request_auth, event.created_at.as_secs())?;

    let target_pubkey =
        PublicKey::from_hex(target_hex).map_err(|e| format!("invalid target pubkey: {e}"))?;
    let target_author = target_pubkey.to_bytes().to_vec();
    let profile = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![KIND_PROFILE as i32]),
            authors: Some(vec![target_author]),
            limit: Some(1),
            global_only: true,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("database error: {e}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "target has no live kind:0 profile".to_string())?;

    if profile.event.pubkey.to_hex() != target_hex {
        return Err("live kind:0 author did not match target".to_string());
    }

    let live_auth = extract_single_auth_tag_json(&profile.event)?;
    let live_owner = verify_auth_tag_owner(&live_auth, target_hex)
        .map_err(|e| format!("invalid live kind:0 auth tag: {e}"))?;
    if live_owner != actor_hex {
        return Err("live kind:0 no longer attests to request signer".to_string());
    }

    Ok(())
}

fn extract_single_auth_tag_json(event: &Event) -> Result<String, String> {
    let mut found: Option<Vec<String>> = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(|s| s.as_str()) != Some("auth") {
            continue;
        }
        if parts.len() != 4 {
            return Err("auth tag must have exactly four elements".to_string());
        }
        if found.is_some() {
            return Err("multiple auth tags".to_string());
        }
        found = Some(parts.iter().map(|s| s.to_string()).collect());
    }

    let parts = found.ok_or_else(|| "missing auth tag".to_string())?;
    serde_json::to_string(&parts).map_err(|e| format!("failed to encode auth tag: {e}"))
}

fn verify_auth_tag_owner(auth_tag_json: &str, target_hex: &str) -> Result<String, String> {
    let target_pubkey =
        PublicKey::from_hex(target_hex).map_err(|e| format!("invalid target pubkey: {e}"))?;
    buzz_sdk::nip_oa::verify_auth_tag(auth_tag_json, &target_pubkey)
        .map(|owner| owner.to_hex())
        .map_err(|e| e.to_string())
}

fn enforce_request_auth_time_bounds(auth_tag_json: &str, created_at: u64) -> Result<(), String> {
    let parts: Vec<String> =
        serde_json::from_str(auth_tag_json).map_err(|e| format!("invalid auth tag json: {e}"))?;
    let conditions = parts
        .get(2)
        .ok_or_else(|| "auth tag missing conditions".to_string())?;

    for clause in conditions.split('&').filter(|clause| !clause.is_empty()) {
        if let Some(bound) = clause.strip_prefix("created_at<") {
            let bound = bound
                .parse::<u64>()
                .map_err(|_| format!("invalid created_at< bound: {bound}"))?;
            if created_at >= bound {
                return Err(format!(
                    "request auth time bound not satisfied: created_at {created_at} >= {bound}"
                ));
            }
        } else if let Some(bound) = clause.strip_prefix("created_at>") {
            let bound = bound
                .parse::<u64>()
                .map_err(|_| format!("invalid created_at> bound: {bound}"))?;
            if created_at <= bound {
                return Err(format!(
                    "request auth time bound not satisfied: created_at {created_at} <= {bound}"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn make_test_event(kind: u16, tags: Vec<Vec<&'static str>>) -> Event {
        let keys = Keys::generate();
        let nostr_tags: Vec<Tag> = tags
            .into_iter()
            .map(|parts| Tag::parse(parts).expect("valid tag"))
            .collect();
        EventBuilder::new(Kind::Custom(kind), "")
            .tags(nostr_tags)
            .sign_with_keys(&keys)
            .expect("signing failed")
    }

    #[test]
    fn extract_single_p_tag_accepts_one_valid_tag() {
        let hex = "a".repeat(64);
        let event = make_test_event(
            9035,
            vec![vec!["p", Box::leak(hex.clone().into_boxed_str())]],
        );
        assert_eq!(extract_single_p_tag_hex(&event), Some(hex));
    }

    #[test]
    fn extract_single_p_tag_rejects_multiple_tags() {
        let event = make_test_event(
            9035,
            vec![
                vec![
                    "p",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ],
                vec![
                    "p",
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ],
            ],
        );
        assert_eq!(extract_single_p_tag_hex(&event), None);
    }

    #[test]
    fn require_single_protected_tag_rejects_missing_or_multiple() {
        let missing = make_test_event(9035, vec![]);
        assert!(require_single_protected_tag(&missing).is_err());

        let multiple = make_test_event(9035, vec![vec!["-"], vec!["-"]]);
        assert!(require_single_protected_tag(&multiple).is_err());
    }

    #[test]
    fn replaced_by_must_be_lowercase_hex_and_not_target() {
        let target = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let same = make_test_event(9035, vec![vec!["replaced-by", target]]);
        assert!(extract_optional_replaced_by(&same, target).is_err());

        let upper = make_test_event(
            9035,
            vec![vec![
                "replaced-by",
                "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
            ]],
        );
        assert!(extract_optional_replaced_by(&upper, target).is_err());
    }

    #[test]
    fn request_time_bounds_ignore_kind_clause() {
        let conditions = "kind=1&created_at>100&created_at<200";
        let auth = serde_json::json!(["auth", "a".repeat(64), conditions, "b".repeat(128)]);
        assert!(enforce_request_auth_time_bounds(&auth.to_string(), 150).is_ok());
        assert!(enforce_request_auth_time_bounds(&auth.to_string(), 100).is_err());
        assert!(enforce_request_auth_time_bounds(&auth.to_string(), 200).is_err());
    }

    async fn test_pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        sqlx::PgPool::connect(&url).await.ok()
    }

    async fn test_state(pool: sqlx::PgPool) -> Option<Arc<AppState>> {
        let db = buzz_db::Db::from_pool(pool.clone());
        let config = crate::config::Config::from_env().ok()?;
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool);
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(buzz_search::SearchConfig {
            url: config.typesense_url.clone(),
            api_key: config.typesense_key.clone(),
            collection: "events".to_string(),
        });
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
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
        Some(Arc::new(state))
    }

    fn auth_tag(owner_keys: &Keys, target_pubkey: &nostr::PublicKey) -> Tag {
        let tag_json = buzz_sdk::nip_oa::compute_auth_tag(owner_keys, target_pubkey, "")
            .expect("compute auth tag");
        buzz_sdk::nip_oa::parse_auth_tag(&tag_json).expect("parse auth tag")
    }

    fn profile_event(target_keys: &Keys, auth_tag: Tag, created_at: u64) -> Event {
        EventBuilder::new(Kind::Metadata, "{}")
            .tags([auth_tag])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(target_keys)
            .expect("sign profile")
    }

    fn owner_archive_request(owner_keys: &Keys, target_hex: &str, auth_tag: Tag) -> Event {
        EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVE_REQUEST as u16), "")
            .tags([
                Tag::parse(["-"]).expect("protected tag"),
                Tag::parse(["p", target_hex]).expect("p tag"),
                auth_tag,
            ])
            .sign_with_keys(owner_keys)
            .expect("sign archive request")
    }

    #[tokio::test]
    async fn owner_archive_rejects_stale_request_after_live_kind0_owner_flip() {
        let Some(pool) = test_pool().await else {
            return;
        };
        if sqlx::query("SELECT 1 FROM archived_identities LIMIT 1")
            .execute(&pool)
            .await
            .is_err()
        {
            return;
        }
        let Some(state) = test_state(pool).await else {
            return;
        };

        let owner_keys = Keys::generate();
        let other_owner_keys = Keys::generate();
        let target_keys = Keys::generate();
        let target_pubkey = target_keys.public_key();
        let target_hex = target_pubkey.to_hex();
        let now = nostr::Timestamp::now().as_secs();

        let live_profile = profile_event(&target_keys, auth_tag(&owner_keys, &target_pubkey), now);
        state
            .db
            .replace_addressable_event(&live_profile, None)
            .await
            .expect("insert initial target kind:0");

        let request_auth = auth_tag(&owner_keys, &target_pubkey);
        let archive_request = owner_archive_request(&owner_keys, &target_hex, request_auth.clone());
        handle_identity_archive_event(&state, &archive_request)
            .await
            .expect("owner archive accepted while live kind:0 attests owner");
        assert!(
            state
                .db
                .is_archived(&target_hex)
                .await
                .expect("is_archived"),
            "first owner archive should mutate archive state"
        );

        let revoked_profile = profile_event(
            &target_keys,
            auth_tag(&other_owner_keys, &target_pubkey),
            now + 1,
        );
        state
            .db
            .replace_addressable_event(&revoked_profile, None)
            .await
            .expect("replace target kind:0");

        let stale_request = owner_archive_request(&owner_keys, &target_hex, request_auth);
        let err = handle_identity_archive_event(&state, &stale_request)
            .await
            .expect_err("stale owner request must be rejected after live kind:0 owner flip");
        assert!(
            err.contains("live kind:0 no longer attests"),
            "unexpected error: {err}"
        );
    }
}
