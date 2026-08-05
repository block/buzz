//! Validated one-way migration from media sidecars to PostgreSQL authority.

use std::collections::BTreeMap;

use buzz_core::tenant::TenantContext;
use buzz_db::protected_publication::MediaPublication;
use buzz_db::protected_visibility::{ProtectedObjectAuthorityState, ProtectedObjectSurface};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::AppState;

const SENTINEL_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct CutoverSentinel {
    format_version: u32,
    community_id: uuid::Uuid,
    surface: String,
    generation: u64,
    imported_objects: u64,
    inventory_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreparationDisposition {
    Begin,
    Resume,
    Verify,
}

fn preparation_disposition(
    authority: &buzz_db::protected_visibility::ProtectedObjectAuthority,
    sentinel: Option<&CutoverSentinel>,
) -> anyhow::Result<PreparationDisposition> {
    match authority.state {
        ProtectedObjectAuthorityState::Legacy => {
            if sentinel.is_some() {
                anyhow::bail!("media cutover sentinel exists but PostgreSQL authority regressed");
            }
            Ok(PreparationDisposition::Begin)
        }
        ProtectedObjectAuthorityState::Importing => {
            if sentinel.is_some_and(|sentinel| sentinel.generation != authority.generation) {
                anyhow::bail!("media resumed import generation conflicts with its sentinel");
            }
            Ok(PreparationDisposition::Resume)
        }
        ProtectedObjectAuthorityState::PostgreSql => {
            let sentinel = sentinel.ok_or_else(|| {
                anyhow::anyhow!("media PostgreSQL authority is missing its sentinel")
            })?;
            validate_authority_snapshot(authority, sentinel)?;
            Ok(PreparationDisposition::Verify)
        }
    }
}

fn sentinel_key(community_id: buzz_core::CommunityId) -> String {
    format!("_authority/{community_id}/media-v1.json")
}

async fn read_sentinel(
    state: &AppState,
    community_id: buzz_core::CommunityId,
) -> anyhow::Result<Option<CutoverSentinel>> {
    let Some(bytes) = state
        .media_storage
        .get_optional(&sentinel_key(community_id))
        .await?
    else {
        return Ok(None);
    };
    let sentinel: CutoverSentinel = serde_json::from_slice(&bytes)?;
    if sentinel.format_version != SENTINEL_FORMAT_VERSION
        || sentinel.community_id != *community_id.as_uuid()
        || sentinel.surface != "media"
    {
        anyhow::bail!("media cutover sentinel does not match its domain and surface");
    }
    validate_digest(&sentinel.inventory_sha256)?;
    Ok(Some(sentinel))
}

async fn create_sentinel(state: &AppState, sentinel: &CutoverSentinel) -> anyhow::Result<()> {
    let community_id = buzz_core::CommunityId::from_uuid(sentinel.community_id);
    let body = serde_json::to_vec(sentinel)?;
    match state
        .media_storage
        .put_create_only(&sentinel_key(community_id), &body, "application/json")
        .await?
    {
        buzz_media::storage::CreateOnlyOutcome::Created => Ok(()),
        buzz_media::storage::CreateOnlyOutcome::AlreadyExists => {
            if read_sentinel(state, community_id).await?.as_ref() == Some(sentinel) {
                Ok(())
            } else {
                anyhow::bail!("media cutover sentinel conflicts with the prepared inventory")
            }
        }
    }
}

/// Prepare one domain's exact, resumable media visibility import before serving.
pub async fn prepare_postgres_authority(
    state: &AppState,
    tenant: &TenantContext,
) -> anyhow::Result<()> {
    if state.restore_protection().is_some() {
        anyhow::bail!(
            "media cutover must complete before the protected restore anchor is provisioned"
        );
    }
    let authority = state
        .db
        .protected_object_authority(tenant.community(), ProtectedObjectSurface::Media)
        .await?;
    let existing_sentinel = read_sentinel(state, tenant.community()).await?;
    let disposition = preparation_disposition(&authority, existing_sentinel.as_ref())?;
    let authority = if disposition == PreparationDisposition::Begin {
        state
            .db
            .begin_protected_object_import(tenant.community(), ProtectedObjectSurface::Media)
            .await?
    } else {
        authority
    };
    if disposition == PreparationDisposition::Verify {
        let sentinel = existing_sentinel.ok_or_else(|| {
            anyhow::anyhow!("media verified authority is missing its cutover sentinel")
        })?;
        return validate_authority_snapshot(&authority, &sentinel);
    }

    let legacy = read_legacy_inventory(state, tenant).await?;
    for publication in legacy.values() {
        let mut transaction = state.db.begin_transaction().await?;
        buzz_db::protected_publication::import_media_publication(
            &mut transaction,
            tenant.community(),
            publication,
        )
        .await?;
        transaction.commit().await?;
    }
    let verified = read_legacy_inventory(state, tenant).await?;
    if verified != legacy {
        anyhow::bail!("media migration inventory changed during verification");
    }
    let postgres = {
        let mut transaction = state.db.begin_transaction().await?;
        let rows = buzz_db::protected_publication::list_media_publications(
            &mut transaction,
            tenant.community(),
        )
        .await?;
        transaction.commit().await?;
        rows.into_iter()
            .map(|publication| (publication.sha256.clone(), publication))
            .collect::<BTreeMap<_, _>>()
    };
    if postgres != legacy {
        anyhow::bail!("media migration inventory parity failed");
    }
    let inventory = media_inventory_digest(&postgres);
    let sentinel = CutoverSentinel {
        format_version: SENTINEL_FORMAT_VERSION,
        community_id: *tenant.community().as_uuid(),
        surface: "media".into(),
        generation: authority.generation,
        imported_objects: postgres.len() as u64,
        inventory_sha256: inventory.clone(),
    };
    if let Some(existing) = existing_sentinel {
        if existing != sentinel {
            anyhow::bail!("media cutover sentinel does not match the resumed import");
        }
    } else {
        create_sentinel(state, &sentinel).await?;
    }
    state
        .db
        .finalize_protected_object_import(
            tenant.community(),
            ProtectedObjectSurface::Media,
            authority.generation,
            postgres.len() as u64,
            &inventory,
        )
        .await?;
    Ok(())
}

/// Require a completed, reconciled authority without advancing migration state.
pub async fn require_reconciled_authority(
    state: &AppState,
    tenant: &TenantContext,
) -> anyhow::Result<()> {
    let authority = state
        .db
        .protected_object_authority(tenant.community(), ProtectedObjectSurface::Media)
        .await?;
    if authority.state != ProtectedObjectAuthorityState::PostgreSql {
        anyhow::bail!("media PostgreSQL authority has not completed preparation");
    }
    let sentinel = read_sentinel(state, tenant.community())
        .await?
        .ok_or_else(|| anyhow::anyhow!("media PostgreSQL authority sentinel is missing"))?;
    validate_authority_snapshot(&authority, &sentinel)
}

/// Refuse a legacy lane after the immutable cutover sentinel exists. This is
/// checked in every mode so a restored pre-cutover database cannot revive
/// stale object-store visibility.
pub async fn require_legacy_sentinel_absent(
    state: &AppState,
    tenant: &TenantContext,
) -> anyhow::Result<()> {
    if read_sentinel(state, tenant.community()).await?.is_some() {
        anyhow::bail!("media legacy authority is permanently unavailable after cutover");
    }
    Ok(())
}

fn validate_authority_snapshot(
    authority: &buzz_db::protected_visibility::ProtectedObjectAuthority,
    sentinel: &CutoverSentinel,
) -> anyhow::Result<()> {
    if authority.state != ProtectedObjectAuthorityState::PostgreSql
        || authority.generation != sentinel.generation
        || authority.imported_objects != Some(sentinel.imported_objects)
        || authority.inventory_sha256.as_deref() != Some(&sentinel.inventory_sha256)
    {
        anyhow::bail!("media PostgreSQL authority and cutover sentinel disagree");
    }
    Ok(())
}

async fn read_legacy_inventory(
    state: &AppState,
    tenant: &TenantContext,
) -> anyhow::Result<BTreeMap<String, MediaPublication>> {
    let prefix = format!("_meta/{}/", tenant.community());
    let mut continuation = None;
    let mut inventory = BTreeMap::new();
    loop {
        let page = state
            .media_storage
            .list_page_with_prefix(prefix.clone(), continuation, 1000)
            .await?;
        for (key, _size) in page.objects {
            let sha256 = key
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(".json"))
                .ok_or_else(|| anyhow::anyhow!("media migration found an invalid sidecar key"))?;
            validate_digest(sha256)?;
            let metadata = state.media_storage.get_sidecar(tenant, sha256).await?;
            validate_extension(&metadata.ext)?;
            let object_key = format!("{sha256}.{}", metadata.ext);
            let head = state
                .media_storage
                .head_with_metadata(&object_key)
                .await?
                .ok_or_else(|| anyhow::anyhow!("media migration blob is missing"))?;
            if head.size != metadata.size {
                anyhow::bail!("media migration blob size does not match sidecar");
            }
            let mut body = state.media_storage.get_stream(&object_key).await?;
            let mut digest = Sha256::new();
            while let Some(chunk) = body.next().await {
                digest.update(chunk?);
            }
            if hex::encode(digest.finalize()) != sha256 {
                anyhow::bail!("media migration blob digest does not match its key");
            }
            let thumbnail_key = format!("{sha256}.thumb.jpg");
            let thumbnail_key = state
                .media_storage
                .head(&thumbnail_key)
                .await?
                .then_some(thumbnail_key);
            let publication = MediaPublication {
                sha256: sha256.to_owned(),
                object_key,
                extension: metadata.ext.clone(),
                mime_type: metadata.mime_type.clone(),
                object_size: metadata.size,
                metadata: serde_json::to_value(metadata)?,
                thumbnail_key,
                publication_version: 1,
            };
            if inventory.insert(sha256.to_owned(), publication).is_some() {
                anyhow::bail!("media migration sidecar inventory contains a duplicate");
            }
        }
        if !page.is_truncated {
            break;
        }
        continuation = page.next_continuation_token;
        if continuation.is_none() {
            anyhow::bail!("media migration listing was truncated without a continuation token");
        }
    }
    Ok(inventory)
}

fn validate_digest(value: &str) -> anyhow::Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
    {
        anyhow::bail!("media migration sidecar digest is invalid");
    }
    Ok(())
}

fn validate_extension(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || value.len() > 16
        || !value
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    {
        anyhow::bail!("media migration sidecar extension is invalid");
    }
    Ok(())
}

fn media_inventory_digest(inventory: &BTreeMap<String, MediaPublication>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"buzz-protected-media-inventory-v1\0");
    for publication in inventory.values() {
        digest.update(publication.sha256.as_bytes());
        digest.update([0]);
        digest.update(publication.object_key.as_bytes());
        digest.update([0]);
        digest.update(publication.mime_type.as_bytes());
        digest.update([0]);
        digest.update(publication.object_size.to_be_bytes());
        digest.update([0]);
        digest.update(serde_json::to_vec(&publication.metadata).unwrap_or_default());
        digest.update([0]);
        if let Some(thumbnail) = &publication.thumbnail_key {
            digest.update(thumbnail.as_bytes());
        }
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::api::git::manifest::{pointer_key, Manifest, MANIFEST_VERSION};
    use crate::api::git::store::Precond;
    use buzz_core::tenant::TenantContext;
    use buzz_media::BlobMeta;

    async fn migration_test_state() -> (Arc<AppState>, TenantContext, sqlx::PgPool) {
        let mut config = crate::config::Config::from_env().expect("test configuration");
        if let Ok(database_url) = std::env::var("BUZZ_TEST_DATABASE_URL") {
            config.database_url = database_url;
        }
        config.redis_url = "redis://127.0.0.1:6379".into();
        let pool = sqlx::PgPool::connect(&config.database_url)
            .await
            .expect("test database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrated test database");
        let community_uuid = uuid::Uuid::new_v4();
        let host = format!("migration-{}.example", community_uuid.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("test community");
        let tenant =
            TenantContext::resolved(buzz_core::CommunityId::from_uuid(community_uuid), host);
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
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        (Arc::new(state), tenant, pool)
    }

    #[test]
    fn strict_legacy_key_components_reject_ambiguous_values() {
        assert!(validate_digest(&"a".repeat(64)).is_ok());
        assert!(validate_digest(&"A".repeat(64)).is_err());
        assert!(validate_extension("webp").is_ok());
        assert!(validate_extension("../jpg").is_err());
        assert!(validate_extension("JPG").is_err());
    }

    #[test]
    fn authority_snapshot_rejects_restore_regression() {
        let sentinel = CutoverSentinel {
            format_version: SENTINEL_FORMAT_VERSION,
            community_id: uuid::Uuid::nil(),
            surface: "media".into(),
            generation: 2,
            imported_objects: 1,
            inventory_sha256: "a".repeat(64),
        };
        let authority = buzz_db::protected_visibility::ProtectedObjectAuthority {
            generation: 1,
            state: ProtectedObjectAuthorityState::Legacy,
            imported_objects: None,
            inventory_sha256: None,
        };
        assert!(validate_authority_snapshot(&authority, &sentinel).is_err());
    }

    #[test]
    fn migration_state_matrix_is_resumable_and_fail_closed() {
        let sentinel = CutoverSentinel {
            format_version: SENTINEL_FORMAT_VERSION,
            community_id: uuid::Uuid::nil(),
            surface: "media".into(),
            generation: 2,
            imported_objects: 1,
            inventory_sha256: "a".repeat(64),
        };
        let authority = |state, generation, imported_objects, inventory_sha256| {
            buzz_db::protected_visibility::ProtectedObjectAuthority {
                generation,
                state,
                imported_objects,
                inventory_sha256,
            }
        };
        assert_eq!(
            preparation_disposition(
                &authority(ProtectedObjectAuthorityState::Legacy, 1, None, None),
                None,
            )
            .unwrap(),
            PreparationDisposition::Begin
        );
        assert_eq!(
            preparation_disposition(
                &authority(ProtectedObjectAuthorityState::Importing, 2, None, None),
                None,
            )
            .unwrap(),
            PreparationDisposition::Resume
        );
        assert_eq!(
            preparation_disposition(
                &authority(ProtectedObjectAuthorityState::Importing, 2, None, None),
                Some(&sentinel),
            )
            .unwrap(),
            PreparationDisposition::Resume
        );
        assert_eq!(
            preparation_disposition(
                &authority(
                    ProtectedObjectAuthorityState::PostgreSql,
                    2,
                    Some(1),
                    Some("a".repeat(64)),
                ),
                Some(&sentinel),
            )
            .unwrap(),
            PreparationDisposition::Verify
        );
        assert!(preparation_disposition(
            &authority(ProtectedObjectAuthorityState::Legacy, 1, None, None),
            Some(&sentinel),
        )
        .is_err());
        assert!(preparation_disposition(
            &authority(ProtectedObjectAuthorityState::Importing, 3, None, None),
            Some(&sentinel),
        )
        .is_err());
        assert!(preparation_disposition(
            &authority(
                ProtectedObjectAuthorityState::PostgreSql,
                2,
                Some(1),
                Some("a".repeat(64)),
            ),
            None,
        )
        .is_err());
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres, Redis, MinIO, and git"]
    async fn populated_git_and_media_cutover_is_validated_resumable_and_one_way() {
        let (state, tenant, pool) = migration_test_state().await;

        let media_bytes = format!("migration-media-{}", uuid::Uuid::new_v4()).into_bytes();
        let media_digest = hex::encode(Sha256::digest(&media_bytes));
        let media_key = format!("{media_digest}.bin");
        state
            .media_storage
            .put(&media_key, &media_bytes, "application/octet-stream")
            .await
            .expect("legacy media blob");
        let mut media_meta = BlobMeta {
            ext: "bin".into(),
            mime_type: "application/octet-stream".into(),
            size: media_bytes.len() as u64 + 1,
            ..BlobMeta::default()
        };
        state
            .media_storage
            .put_sidecar(&tenant, &media_digest, &media_meta)
            .await
            .expect("corrupt legacy sidecar fixture");

        // Import starts durably before inventory validation. A corrupt legacy
        // row must leave that checkpoint resumable and never create authority.
        assert!(
            prepare_postgres_authority(&state, &tenant)
                .await
                .expect_err("corrupt sidecar must fail")
                .to_string()
                .contains("size does not match"),
            "corruption must be diagnosed before cutover"
        );
        assert_eq!(
            state
                .db
                .protected_object_authority(
                    tenant.community(),
                    buzz_db::protected_visibility::ProtectedObjectSurface::Media,
                )
                .await
                .expect("failed import state")
                .state,
            ProtectedObjectAuthorityState::Importing
        );
        assert!(
            read_sentinel(&state, tenant.community())
                .await
                .expect("failed import sentinel probe")
                .is_none(),
            "corrupt inventory must not create a cutover sentinel"
        );

        media_meta.size = media_bytes.len() as u64;
        state
            .media_storage
            .put_sidecar(&tenant, &media_digest, &media_meta)
            .await
            .expect("repair sidecar fixture");
        prepare_postgres_authority(&state, &tenant)
            .await
            .expect("resume validated media import");

        let owner = hex::encode(Sha256::digest(uuid::Uuid::new_v4().as_bytes()));
        let repo_id = format!("migration-{}", uuid::Uuid::new_v4().simple());
        state
            .db
            .reserve_repo_name(tenant.community(), &repo_id, &owner)
            .await
            .expect("legacy repo reservation");
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            head: "refs/heads/main".into(),
            refs: BTreeMap::new(),
            packs: Vec::new(),
            parent: None,
        };
        manifest.validate().expect("valid empty manifest");
        let manifest_key = state
            .git_store
            .put_manifest(&manifest.canonical_bytes().expect("manifest bytes"))
            .await
            .expect("legacy manifest");
        let manifest_digest = manifest_key
            .strip_prefix("manifests/")
            .expect("manifest digest")
            .to_owned();
        state
            .git_store
            .put_pointer(
                &pointer_key(tenant.community(), &owner, &repo_id),
                manifest_digest.as_bytes(),
                Precond::IfNoneMatchStar,
            )
            .await
            .expect("legacy pointer");

        // A transaction-owned announcement with no legacy pointer remains a
        // valid first-push reservation after cutover.
        let unpublished_repo = format!("unpublished-{}", uuid::Uuid::new_v4().simple());
        let announcement_id = hex::encode(Sha256::digest(uuid::Uuid::new_v4().as_bytes()));
        let owner_bytes = hex::decode(&owner).expect("owner bytes");
        sqlx::query(
            "INSERT INTO events \
             (community_id, id, pubkey, created_at, kind, tags, content, sig, d_tag) \
             VALUES ($1, $2, $3, clock_timestamp(), 30617, $4, '', $5, $6)",
        )
        .bind(tenant.community().as_uuid())
        .bind(hex::decode(&announcement_id).expect("announcement bytes"))
        .bind(&owner_bytes)
        .bind(serde_json::json!([["d", unpublished_repo.clone()]]))
        .bind(vec![0_u8; 64])
        .bind(&unpublished_repo)
        .execute(&pool)
        .await
        .expect("protected repository announcement");
        sqlx::query(
            "INSERT INTO git_repo_names \
             (community_id, repo_id, owner_pubkey, publication_origin) \
             VALUES ($1, $2, $3, 'protected_unpublished')",
        )
        .bind(tenant.community().as_uuid())
        .bind(&unpublished_repo)
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("protected first-push reservation");

        crate::api::git::migration::prepare_postgres_authority(&state, &tenant)
            .await
            .expect("validated Git import");

        let media_publication = state
            .db
            .media_publication(tenant.community(), &media_digest)
            .await
            .expect("media publication query")
            .expect("imported media publication");
        assert_eq!(media_publication.object_key, media_key);
        assert_eq!(media_publication.object_size, media_bytes.len() as u64);
        let git_publication = state
            .db
            .git_publication(tenant.community(), &repo_id, &owner)
            .await
            .expect("Git publication query")
            .expect("imported Git publication");
        assert_eq!(git_publication.manifest_sha256, manifest_digest);
        assert!(state
            .db
            .git_publication(tenant.community(), &unpublished_repo, &owner)
            .await
            .expect("first-push publication query")
            .is_none());

        // The first protected push commits only the PostgreSQL publication.
        // It must remain readable through the immutable manifest after a
        // migration verification restart, without reviving a legacy pointer.
        let policy = buzz_db::protected_publication::GitPolicyCommitFence {
            announcement_id,
            channel_id: None,
            grant: buzz_db::protected_publication::GitPolicyGrant::RepoOwner,
        };
        let mut git_transaction = state.db.begin_transaction().await.expect("Git transaction");
        let first_push = buzz_db::protected_publication::compare_and_publish_git(
            &mut git_transaction,
            buzz_db::protected_publication::GitPublicationRequest {
                community_id: tenant.community(),
                repo_id: &unpublished_repo,
                owner_pubkey: &owner,
                expected: None,
                manifest_sha256: &manifest_digest,
                pusher_pubkey: &owner_bytes,
                policy: &policy,
            },
        )
        .await
        .expect("first protected push");
        git_transaction.commit().await.expect("commit first push");
        assert!(matches!(
            first_push,
            buzz_db::protected_publication::GitPublicationOutcome::Published(
                buzz_db::protected_publication::GitPublication {
                    publication_version: 1,
                    ..
                }
            )
        ));
        let first_push_publication = state
            .db
            .git_publication(tenant.community(), &unpublished_repo, &owner)
            .await
            .expect("first-push read")
            .expect("first push is PostgreSQL-authoritative");
        assert_eq!(first_push_publication.manifest_sha256, manifest_digest);
        assert_eq!(
            crate::api::git::hydrate::load_manifest_by_digest(
                &state.git_store,
                &first_push_publication.manifest_sha256,
            )
            .await
            .expect("published manifest read"),
            manifest
        );
        crate::api::git::hydrate::hydrate_for_published_read(
            &state.git_store,
            &first_push_publication.manifest_sha256,
            crate::api::git::hydrate::HydrationOptions {
                pack_cache: &state.git_pack_cache,
                scratch_dir: &state.config.git_repo_path,
                max_pack_bytes: state.config.git_max_pack_bytes,
                max_repo_bytes: state.config.git_max_repo_bytes,
            },
        )
        .await
        .expect("first protected publication hydrates for read");
        assert!(state
            .git_store
            .get_pointer(&pointer_key(tenant.community(), &owner, &unpublished_repo,))
            .await
            .expect("legacy first-push pointer probe")
            .is_none());

        // A protected media publication likewise uses PostgreSQL for
        // visibility while the immutable object remains in object storage.
        let protected_media_bytes =
            format!("protected-media-{}", uuid::Uuid::new_v4()).into_bytes();
        let protected_media_digest = hex::encode(Sha256::digest(&protected_media_bytes));
        let protected_media_key = format!("{protected_media_digest}.bin");
        state
            .media_storage
            .put(
                &protected_media_key,
                &protected_media_bytes,
                "application/octet-stream",
            )
            .await
            .expect("protected media blob");
        let sidecar_key =
            buzz_media::MediaStorage::ctx_sidecar_key(&tenant, &protected_media_digest);
        assert!(!state
            .media_storage
            .head(&sidecar_key)
            .await
            .expect("protected sidecar probe"));
        let protected_media = buzz_db::protected_publication::MediaPublication {
            sha256: protected_media_digest.clone(),
            object_key: protected_media_key.clone(),
            extension: "bin".into(),
            mime_type: "application/octet-stream".into(),
            object_size: protected_media_bytes.len() as u64,
            metadata: serde_json::json!({"synthetic": true}),
            thumbnail_key: None,
            publication_version: 1,
        };
        let mut media_transaction = state
            .db
            .begin_transaction()
            .await
            .expect("media transaction");
        buzz_db::protected_publication::publish_media(
            &mut media_transaction,
            tenant.community(),
            &protected_media,
        )
        .await
        .expect("protected media publication");
        media_transaction
            .commit()
            .await
            .expect("commit media publication");
        assert_eq!(
            state
                .db
                .media_publication(tenant.community(), &protected_media_digest)
                .await
                .expect("protected media read")
                .expect("protected media is PostgreSQL-authoritative")
                .object_key,
            protected_media_key
        );
        assert_eq!(
            state
                .media_storage
                .get(&protected_media_key)
                .await
                .expect("protected object read"),
            protected_media_bytes
        );
        let (resolved_mime, resolved_key) = crate::api::media::resolve_visible_media(
            &state,
            &tenant,
            &format!("{protected_media_digest}.bin"),
            &None,
        )
        .await
        .expect("non-Enforce mode retains PostgreSQL-authoritative visibility");
        assert_eq!(resolved_mime, "application/octet-stream");
        assert_eq!(resolved_key, protected_media_key);
        assert!(!state
            .media_storage
            .head(&sidecar_key)
            .await
            .expect("protected sidecar recheck"));

        // Completed imports are exact idempotent verification passes.
        prepare_postgres_authority(&state, &tenant)
            .await
            .expect("media verification retry");
        crate::api::git::migration::prepare_postgres_authority(&state, &tenant)
            .await
            .expect("Git verification retry");
        require_reconciled_authority(&state, &tenant)
            .await
            .expect("media reconciled");
        crate::api::git::migration::require_reconciled_authority(&state, &tenant)
            .await
            .expect("Git reconciled");
        assert!(require_legacy_sentinel_absent(&state, &tenant)
            .await
            .is_err());
        assert!(
            crate::api::git::migration::require_legacy_sentinel_absent(&state, &tenant)
                .await
                .is_err()
        );

        // Simulate restoring only the database to a pre-cutover state. The
        // immutable object-store sentinel must force domain denial rather than
        // reviving the legacy lane or creating split-brain visibility.
        sqlx::query(
            "UPDATE protected_object_authority \
             SET state = 'legacy', generation = 1, imported_objects = 0, \
                 inventory_sha256 = NULL, started_at = NULL, completed_at = NULL \
             WHERE community_id = $1 AND surface IN ('git', 'media')",
        )
        .bind(tenant.community().as_uuid())
        .execute(&pool)
        .await
        .expect("simulate database restore");
        assert!(require_reconciled_authority(&state, &tenant).await.is_err());
        assert!(
            crate::api::git::migration::require_reconciled_authority(&state, &tenant)
                .await
                .is_err()
        );
        assert!(require_legacy_sentinel_absent(&state, &tenant)
            .await
            .is_err());
        assert!(
            crate::api::git::migration::require_legacy_sentinel_absent(&state, &tenant)
                .await
                .is_err()
        );
    }
}
