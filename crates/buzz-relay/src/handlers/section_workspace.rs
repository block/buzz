//! NIP-SW revision-zero owner import.

use super::event::dispatch_persistent_event;
use super::ingest::{IngestError, IngestResult};
use crate::state::AppState;
use buzz_core::kind::{KIND_SECTION_WORKSPACE_IMPORT, KIND_SECTION_WORKSPACE_PROJECTION};
use buzz_core::section_workspace::{validate_import_tags, Import};
use buzz_core::tenant::TenantContext;
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp};
use sqlx::Row;
use std::sync::Arc;
/// Validate and atomically persist one owner import and its owner projection.
pub async fn handle_import(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<IngestResult, IngestError> {
    require_enabled(state.config.section_workspace_import_enabled)?;
    let owner = event.pubkey.to_hex();
    let import: Import = serde_json::from_str(&event.content)
        .map_err(|error| IngestError::Rejected(format!("invalid: import content: {error}")))?;
    import
        .validate()
        .map_err(|error| IngestError::Rejected(format!("invalid: {error}")))?;
    validate_import_tags(event.tags.as_slice(), &owner, import.action_id)
        .map_err(|error| IngestError::Rejected(format!("invalid: {error}")))?;
    let coordinate = format!("{owner}:{owner}");
    let projection_content = serde_json::to_string(&import.owner_projection(owner.clone()))
        .map_err(|error| IngestError::Internal(format!("error: serialize projection: {error}")))?;
    let projection = EventBuilder::new(
        Kind::Custom(KIND_SECTION_WORKSPACE_PROJECTION as u16),
        projection_content,
    )
    .tags([
        Tag::parse(["d", coordinate.as_str()]).map_err(|error| {
            IngestError::Internal(format!("error: build projection d tag: {error}"))
        })?,
        Tag::parse(["p", owner.as_str()]).map_err(|error| {
            IngestError::Internal(format!("error: build projection p tag: {error}"))
        })?,
    ])
    .custom_created_at(Timestamp::now())
    .sign_with_keys(&state.relay_keypair)
    .map_err(|error| IngestError::Internal(format!("error: sign projection: {error}")))?;
    let duplicate = persist_import(tenant, state, event, &import, &projection, &coordinate).await?;
    if duplicate {
        return Ok(IngestResult {
            event_id: event.id.to_hex(),
            accepted: true,
            message: "duplicate: exact import replay".into(),
        });
    }
    let stored = state
        .db
        .get_event_by_id(tenant.community(), projection.id.as_bytes())
        .await
        .map_err(|error| IngestError::Internal(format!("error: load projection: {error}")))?
        .ok_or_else(|| IngestError::Internal("error: committed projection is missing".into()))?;
    dispatch_persistent_event(
        tenant,
        state,
        &stored,
        KIND_SECTION_WORKSPACE_PROJECTION,
        &state.relay_keypair.public_key().to_hex(),
        None,
    )
    .await;
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: String::new(),
    })
}
fn require_enabled(enabled: bool) -> Result<(), IngestError> {
    if enabled {
        Ok(())
    } else {
        Err(IngestError::Rejected(
            "restricted: section workspace import is disabled".into(),
        ))
    }
}
async fn persist_import(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
    import: &Import,
    projection: &Event,
    coordinate: &str,
) -> Result<bool, IngestError> {
    let mut tx = state
        .db
        .begin_transaction()
        .await
        .map_err(|error| IngestError::Internal(format!("error: begin import: {error}")))?;
    buzz_deletion::store(&state.db)
        .guard_transaction(&mut tx, tenant.community())
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: community writes are fenced: {error}"))
        })?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(import_lock(
            *tenant.community().as_uuid(),
            event.pubkey.as_bytes(),
        ))
        .execute(tx.as_mut())
        .await
        .map_err(|error| IngestError::Internal(format!("error: lock import: {error}")))?;
    let existing = sqlx::query(
        "SELECT id FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
         LIMIT 1",
    )
    .bind(tenant.community().as_uuid())
    .bind(KIND_SECTION_WORKSPACE_IMPORT as i32)
    .bind(event.pubkey.as_bytes())
    .fetch_optional(tx.as_mut())
    .await
    .map_err(|error| IngestError::Internal(format!("error: query existing import: {error}")))?;
    if let Some(row) = existing {
        let id: Vec<u8> = row.get("id");
        if id.as_slice() == event.id.as_bytes() {
            return Ok(true);
        }
        return Err(IngestError::Rejected(
            "conflict: section workspace has already been imported".into(),
        ));
    }
    let source_id = hex::decode(&import.source_event_id)
        .map_err(|_| IngestError::Rejected("invalid: source_event_id".into()))?;
    let source_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND kind=$3 \
         AND pubkey=$4 AND deleted_at IS NULL)",
    )
    .bind(tenant.community().as_uuid())
    .bind(source_id)
    .bind(buzz_core::kind::KIND_READ_STATE as i32)
    .bind(event.pubkey.as_bytes())
    .fetch_one(tx.as_mut())
    .await
    .map_err(|error| IngestError::Internal(format!("error: validate legacy source: {error}")))?;
    if !source_exists {
        return Err(IngestError::Rejected(
            "invalid: legacy source is not owner-authored read state in this community".into(),
        ));
    }
    let channel_ids: Vec<uuid::Uuid> = import.assignments.iter().map(|a| a.channel_id).collect();
    let valid_channels: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channels WHERE community_id=$1 AND id=ANY($2) \
         AND deleted_at IS NULL",
    )
    .bind(tenant.community().as_uuid())
    .bind(&channel_ids)
    .fetch_one(tx.as_mut())
    .await
    .map_err(|error| IngestError::Internal(format!("error: validate channels: {error}")))?;
    if valid_channels as usize != channel_ids.len() {
        return Err(IngestError::Rejected(
            "invalid: assignment channel does not exist in this community".into(),
        ));
    }
    insert_event(&mut tx, tenant, event, None).await?;
    insert_event(&mut tx, tenant, projection, Some(coordinate)).await?;
    insert_single_mention(&mut tx, tenant, event, &event.pubkey.to_hex()).await?;
    insert_single_mention(&mut tx, tenant, projection, &event.pubkey.to_hex()).await?;
    tx.commit()
        .await
        .map_err(|error| IngestError::Internal(format!("error: commit import: {error}")))?;
    Ok(false)
}
async fn insert_event(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: &TenantContext,
    event: &Event,
    d_tag: Option<&str>,
) -> Result<(), IngestError> {
    let tags = serde_json::to_value(&event.tags)
        .map_err(|error| IngestError::Internal(format!("error: serialize tags: {error}")))?;
    let created_at = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or_else(|| IngestError::Rejected("invalid: timestamp".into()))?;
    let inserted = sqlx::query(
        "INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id,d_tag) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,NOW(),NULL,$9) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.community().as_uuid())
    .bind(event.id.as_bytes())
    .bind(event.pubkey.as_bytes())
    .bind(created_at)
    .bind(event.kind.as_u16() as i32)
    .bind(tags)
    .bind(&event.content)
    .bind(event.sig.serialize().as_slice())
    .bind(d_tag)
    .execute(tx.as_mut())
    .await
    .map_err(|error| IngestError::Internal(format!("error: persist import event: {error}")))?;
    if inserted.rows_affected() != 1 {
        return Err(IngestError::Internal(
            "error: unexpected event-id collision".into(),
        ));
    }
    Ok(())
}
async fn insert_single_mention(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: &TenantContext,
    event: &Event,
    reader: &str,
) -> Result<(), IngestError> {
    let created_at = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .ok_or_else(|| IngestError::Rejected("invalid: timestamp".into()))?;
    sqlx::query(
        "INSERT INTO event_mentions \
         (community_id,pubkey_hex,event_id,event_created_at,channel_id,event_kind) \
         VALUES ($1,$2,$3,$4,NULL,$5) ON CONFLICT DO NOTHING",
    )
    .bind(tenant.community().as_uuid())
    .bind(reader)
    .bind(event.id.as_bytes())
    .bind(created_at)
    .bind(event.kind.as_u16() as i32)
    .execute(tx.as_mut())
    .await
    .map_err(|error| IngestError::Internal(format!("error: index private event: {error}")))?;
    Ok(())
}
fn import_lock(community: uuid::Uuid, owner: &[u8]) -> i64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in community.as_bytes().iter().chain(owner) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}
#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{
        channel::{ChannelType, ChannelVisibility},
        section_workspace::{ImportAssignment, ImportSection, Projection, VERSION},
        tenant::CommunityId,
    };
    use nostr::Keys;
    #[test]
    fn disabled_gate_rejects_before_database_work() {
        assert!(
            matches!(require_enabled(false), Err(IngestError::Rejected(message)) if message.contains("disabled"))
        );
        require_enabled(true).unwrap();
    }
    async fn state(pool: sqlx::PgPool) -> Option<Arc<AppState>> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.section_workspace_import_enabled = true;
        config.redis_url = "redis://127.0.0.1:6379".into();
        let redis = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        redis::cmd("PING")
            .query_async::<String>(&mut redis.get().await.ok()?)
            .await
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis.clone())
                .await
                .ok()?,
        );
        let db = buzz_db::Db::from_pool(pool.clone());
        let workflow = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let (state, _) = AppState::new(
            config,
            db,
            redis,
            buzz_audit::AuditService::new(pool.clone()),
            pubsub,
            buzz_auth::AuthService::new(crate::config::Config::from_env().ok()?.auth),
            buzz_search::SearchService::new(pool),
            workflow,
            Keys::generate(),
            buzz_media::MediaStorage::new(&crate::config::Config::from_env().ok()?.media).ok()?,
        );
        Some(Arc::new(state))
    }
    fn coordinate_for_test(keys: &Keys) -> String {
        let owner = keys.public_key().to_hex();
        format!("{owner}:{owner}")
    }

    fn command(keys: &Keys, import: &Import) -> Event {
        EventBuilder::new(
            Kind::Custom(KIND_SECTION_WORKSPACE_IMPORT as u16),
            serde_json::to_string(import).unwrap(),
        )
        .tags([
            Tag::parse(["p", &keys.public_key().to_hex()]).unwrap(),
            Tag::parse(["action", &import.action_id.to_string()]).unwrap(),
        ])
        .allow_self_tagging()
        .sign_with_keys(keys)
        .unwrap()
    }
    #[tokio::test]
    async fn owner_import_is_atomic_private_and_idempotent() {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz@localhost:5432/buzz".into());
        let Ok(pool) = sqlx::PgPool::connect(&url).await else {
            return;
        };
        let Some(state) = state(pool.clone()).await else {
            return;
        };
        let id = uuid::Uuid::new_v4();
        let tenant = TenantContext::resolved(
            CommunityId::from_uuid(id),
            format!("sections-{}.example", id.simple()),
        );
        sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
            .bind(id)
            .bind(tenant.host())
            .execute(&pool)
            .await
            .unwrap();
        let owner = Keys::generate();
        let channel = state
            .db
            .create_channel(
                tenant.community(),
                "imported",
                ChannelType::Stream,
                ChannelVisibility::Open,
                None,
                owner.public_key().as_bytes(),
                None,
            )
            .await
            .unwrap();
        let source = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
            "legacy",
        )
        .sign_with_keys(&owner)
        .unwrap();
        state
            .db
            .insert_event(tenant.community(), &source, None)
            .await
            .unwrap();
        let section = uuid::Uuid::new_v4();
        let import = Import {
            version: VERSION,
            action_id: uuid::Uuid::new_v4(),
            source_event_id: source.id.to_hex(),
            source_hash: "33".repeat(32),
            key_epoch: 1,
            owner_key_envelope: "owner-envelope".into(),
            sections: vec![ImportSection {
                id: section,
                rank: 0,
                encrypted_label: "encrypted".into(),
                encrypted_icon: None,
            }],
            assignments: vec![ImportAssignment {
                channel_id: channel.id,
                section_id: section,
            }],
        };
        let event = command(&owner, &import);
        assert!(
            handle_import(&tenant, &state, &event)
                .await
                .unwrap()
                .accepted
        );
        assert!(handle_import(&tenant, &state, &event)
            .await
            .unwrap()
            .message
            .starts_with("duplicate:"));
        assert!(state
            .db
            .get_event_by_id(tenant.community(), event.id.as_bytes())
            .await
            .unwrap()
            .is_some());
        assert!(buzz_core::filter::reader_authorized_for_event(
            &event,
            &owner.public_key().to_hex()
        ));
        assert!(!buzz_core::filter::reader_authorized_for_event(
            &event,
            &Keys::generate().public_key().to_hex()
        ));
        let rows = state
            .db
            .query_events(&buzz_db::EventQuery {
                kinds: Some(vec![KIND_SECTION_WORKSPACE_PROJECTION as i32]),
                global_only: true,
                ..buzz_db::EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let projection = &rows[0].event;
        assert!(sqlx::query_scalar::<_, bool>(
            "SELECT search_tsv IS NULL FROM events WHERE community_id=$1 AND id=$2"
        )
        .bind(id)
        .bind(projection.id.as_bytes())
        .fetch_one(&pool)
        .await
        .unwrap());
        assert_eq!(
            serde_json::from_str::<Projection>(&projection.content)
                .unwrap()
                .revision,
            1
        );
        assert!(buzz_core::filter::reader_authorized_for_event(
            projection,
            &owner.public_key().to_hex()
        ));
        assert!(!buzz_core::filter::reader_authorized_for_event(
            projection,
            &Keys::generate().public_key().to_hex()
        ));

        for target in [&event, projection] {
            let deletion = EventBuilder::new(Kind::Custom(5), "")
                .tags([Tag::parse(["e", &target.id.to_hex()]).unwrap()])
                .sign_with_keys(&owner)
                .unwrap();
            let error = super::super::side_effects::validate_standard_deletion_event(
                &tenant, &deletion, &state,
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("immutable"));
        }
        let address = format!(
            "{}:{}:{}",
            KIND_SECTION_WORKSPACE_PROJECTION,
            projection.pubkey.to_hex(),
            coordinate_for_test(&owner)
        );
        let address_deletion = EventBuilder::new(Kind::Custom(5), "")
            .tags([Tag::parse(["a", &address]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();
        let error = super::super::side_effects::validate_standard_deletion_event(
            &tenant,
            &address_deletion,
            &state,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("immutable"));

        state
            .db
            .soft_delete_event(tenant.community(), event.id.as_bytes())
            .await
            .unwrap();
        assert!(handle_import(&tenant, &state, &event)
            .await
            .unwrap()
            .message
            .starts_with("duplicate:"));
        let mut conflicting = import.clone();
        conflicting.action_id = uuid::Uuid::new_v4();
        assert!(
            matches!(handle_import(&tenant, &state, &command(&owner, &conflicting)).await, Err(IngestError::Rejected(message)) if message.starts_with("conflict:"))
        );
        let fresh = Keys::generate();
        let fresh_source = EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16),
            "legacy",
        )
        .sign_with_keys(&fresh)
        .unwrap();
        state
            .db
            .insert_event(tenant.community(), &fresh_source, None)
            .await
            .unwrap();
        let mut invalid = import;
        invalid.action_id = uuid::Uuid::new_v4();
        invalid.source_event_id = fresh_source.id.to_hex();
        invalid.assignments[0].channel_id = uuid::Uuid::new_v4();
        let rejected = command(&fresh, &invalid);
        assert!(handle_import(&tenant, &state, &rejected).await.is_err());
        assert!(state
            .db
            .get_event_by_id(tenant.community(), rejected.id.as_bytes())
            .await
            .unwrap()
            .is_none());
    }
}
