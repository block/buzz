//! Validated one-way migration from legacy Git pointers to PostgreSQL authority.

use std::collections::BTreeMap;

use buzz_core::tenant::TenantContext;
use buzz_db::protected_visibility::{ProtectedObjectAuthorityState, ProtectedObjectSurface};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::git::hydrate::{hydrate_for_published_read, load_manifest_by_digest};
use crate::api::git::manifest::pointer_key;
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
                anyhow::bail!("Git cutover sentinel exists but PostgreSQL authority regressed");
            }
            Ok(PreparationDisposition::Begin)
        }
        ProtectedObjectAuthorityState::Importing => {
            if sentinel.is_some_and(|sentinel| sentinel.generation != authority.generation) {
                anyhow::bail!("Git resumed import generation conflicts with its sentinel");
            }
            Ok(PreparationDisposition::Resume)
        }
        ProtectedObjectAuthorityState::PostgreSql => {
            let sentinel = sentinel.ok_or_else(|| {
                anyhow::anyhow!("Git PostgreSQL authority is missing its sentinel")
            })?;
            validate_authority_snapshot(authority, sentinel)?;
            Ok(PreparationDisposition::Verify)
        }
    }
}

fn sentinel_key(community_id: buzz_core::CommunityId) -> String {
    format!("_authority/{community_id}/git-v1.json")
}

async fn read_sentinel(
    state: &AppState,
    community_id: buzz_core::CommunityId,
) -> anyhow::Result<Option<CutoverSentinel>> {
    let key = sentinel_key(community_id);
    let Some((_etag, bytes)) = state.git_store.get_pointer(&key).await? else {
        return Ok(None);
    };
    let sentinel: CutoverSentinel = serde_json::from_slice(&bytes)?;
    if sentinel.format_version != SENTINEL_FORMAT_VERSION
        || sentinel.community_id != *community_id.as_uuid()
        || sentinel.surface != "git"
    {
        anyhow::bail!("Git cutover sentinel does not match its domain and surface");
    }
    validate_digest(&sentinel.inventory_sha256)?;
    Ok(Some(sentinel))
}

async fn create_sentinel(state: &AppState, sentinel: &CutoverSentinel) -> anyhow::Result<()> {
    let key = sentinel_key(buzz_core::CommunityId::from_uuid(sentinel.community_id));
    let body = serde_json::to_vec(sentinel)?;
    match state
        .git_store
        .put_pointer(
            &key,
            &body,
            crate::api::git::store::Precond::IfNoneMatchStar,
        )
        .await?
    {
        crate::api::git::store::CasOutcome::Won(_) => Ok(()),
        crate::api::git::store::CasOutcome::LostRace => {
            let existing = read_sentinel(
                state,
                buzz_core::CommunityId::from_uuid(sentinel.community_id),
            )
            .await?;
            if existing.as_ref() == Some(sentinel) {
                Ok(())
            } else {
                anyhow::bail!("Git cutover sentinel conflicts with the prepared inventory")
            }
        }
    }
}

/// Prepare one domain's exact, resumable Git visibility import before serving.
pub async fn prepare_postgres_authority(
    state: &AppState,
    tenant: &TenantContext,
) -> anyhow::Result<()> {
    if state.restore_protection().is_some() {
        anyhow::bail!(
            "Git cutover must complete before the protected restore anchor is provisioned"
        );
    }
    let authority = state
        .db
        .protected_object_authority(tenant.community(), ProtectedObjectSurface::Git)
        .await?;
    let existing_sentinel = read_sentinel(state, tenant.community()).await?;
    let disposition = preparation_disposition(&authority, existing_sentinel.as_ref())?;
    let authority = if disposition == PreparationDisposition::Begin {
        state
            .db
            .begin_protected_object_import(tenant.community(), ProtectedObjectSurface::Git)
            .await?
    } else {
        authority
    };
    if disposition == PreparationDisposition::Verify {
        let sentinel = existing_sentinel.ok_or_else(|| {
            anyhow::anyhow!("Git verified authority is missing its cutover sentinel")
        })?;
        return validate_authority_snapshot(&authority, &sentinel);
    }

    let reservations = {
        let mut transaction = state.db.begin_transaction().await?;
        let reservations = buzz_db::protected_publication::list_git_repo_reservations(
            &mut transaction,
            tenant.community(),
        )
        .await?;
        transaction.commit().await?;
        reservations
    };

    let reservation_map = reservations
        .iter()
        .map(|(repo, owner, origin)| (repo.clone(), (owner.clone(), origin.clone())))
        .collect::<BTreeMap<String, (String, String)>>();
    let mut legacy = BTreeMap::new();
    for (repo_id, owner, origin) in reservations {
        let pointer = pointer_key(tenant.community(), &owner, &repo_id);
        let Some((_etag, body)) = state.git_store.get_pointer(&pointer).await? else {
            if origin == "protected_unpublished" {
                // A transaction-owned Enforce announcement intentionally has
                // no legacy pointer and may take its first push after cutover.
                continue;
            }
            anyhow::bail!("Git migration found a legacy reservation without a pointer");
        };
        if origin != "legacy" {
            anyhow::bail!("Git migration found a protected-unpublished reservation with a pointer");
        }
        let digest = std::str::from_utf8(&body)
            .map_err(|_| anyhow::anyhow!("Git migration pointer is not UTF-8"))?
            .trim()
            .to_owned();
        validate_digest(&digest)?;
        let _manifest = load_manifest_by_digest(&state.git_store, &digest).await?;
        // Fully materialize every referenced pack and ref. A digest-valid
        // manifest with a missing or corrupt child must never pass cutover.
        let hydrated = hydrate_for_published_read(
            &state.git_store,
            &digest,
            crate::api::git::hydrate::HydrationOptions {
                pack_cache: &state.git_pack_cache,
                scratch_dir: &state.config.git_repo_path,
                max_pack_bytes: state.config.git_max_pack_bytes,
                max_repo_bytes: state.config.git_max_repo_bytes,
            },
        )
        .await?;
        drop(hydrated);

        let mut transaction = state.db.begin_transaction().await?;
        buzz_db::protected_publication::import_git_publication(
            &mut transaction,
            tenant.community(),
            &repo_id,
            &owner,
            &digest,
        )
        .await?;
        transaction.commit().await?;
        legacy.insert(repo_id, (owner, digest));
    }

    // Re-read every pointer after import. The transaction-held legacy writer
    // fence guarantees this set cannot change after `importing` began; this
    // second pass detects incompatible writers and object corruption loudly.
    for (repo_id, (owner, expected)) in &legacy {
        let pointer = pointer_key(tenant.community(), owner, repo_id);
        let Some((_etag, body)) = state.git_store.get_pointer(&pointer).await? else {
            anyhow::bail!("Git migration pointer disappeared during verification");
        };
        let actual = std::str::from_utf8(&body)
            .map_err(|_| anyhow::anyhow!("Git migration pointer is not UTF-8"))?
            .trim();
        if actual != expected {
            anyhow::bail!("Git migration pointer changed during verification");
        }
    }
    let verified_reservations = {
        let mut transaction = state.db.begin_transaction().await?;
        let rows = buzz_db::protected_publication::list_git_repo_reservations(
            &mut transaction,
            tenant.community(),
        )
        .await?;
        transaction.commit().await?;
        rows.into_iter()
            .map(|(repo, owner, origin)| (repo, (owner, origin)))
            .collect::<BTreeMap<_, _>>()
    };
    if verified_reservations != reservation_map {
        anyhow::bail!("Git migration reservation inventory changed during verification");
    }
    let postgres = {
        let mut transaction = state.db.begin_transaction().await?;
        let rows = buzz_db::protected_publication::list_git_publications(
            &mut transaction,
            tenant.community(),
        )
        .await?;
        transaction.commit().await?;
        rows.into_iter()
            .map(|(repo, owner, digest)| (repo, (owner, digest)))
            .collect::<BTreeMap<_, _>>()
    };
    if postgres != legacy {
        anyhow::bail!("Git migration inventory parity failed");
    }
    let inventory = git_inventory_digest(&postgres);
    let sentinel = CutoverSentinel {
        format_version: SENTINEL_FORMAT_VERSION,
        community_id: *tenant.community().as_uuid(),
        surface: "git".into(),
        generation: authority.generation,
        imported_objects: postgres.len() as u64,
        inventory_sha256: inventory.clone(),
    };
    if let Some(existing) = existing_sentinel {
        if existing != sentinel {
            anyhow::bail!("Git cutover sentinel does not match the resumed import");
        }
    } else {
        create_sentinel(state, &sentinel).await?;
    }
    state
        .db
        .finalize_protected_object_import(
            tenant.community(),
            ProtectedObjectSurface::Git,
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
        .protected_object_authority(tenant.community(), ProtectedObjectSurface::Git)
        .await?;
    if authority.state != ProtectedObjectAuthorityState::PostgreSql {
        anyhow::bail!("Git PostgreSQL authority has not completed preparation");
    }
    let sentinel = read_sentinel(state, tenant.community())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Git PostgreSQL authority sentinel is missing"))?;
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
        anyhow::bail!("Git legacy authority is permanently unavailable after cutover");
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
        anyhow::bail!("Git PostgreSQL authority and cutover sentinel disagree");
    }
    Ok(())
}

fn validate_digest(value: &str) -> anyhow::Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
    {
        anyhow::bail!("Git migration pointer digest is invalid");
    }
    Ok(())
}

fn git_inventory_digest(inventory: &BTreeMap<String, (String, String)>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"buzz-protected-git-inventory-v1\0");
    for (repo, (owner, manifest)) in inventory {
        digest.update(repo.as_bytes());
        digest.update([0]);
        digest.update(owner.as_bytes());
        digest.update([0]);
        digest.update(manifest.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_digest_is_order_independent_and_value_sensitive() {
        let mut first = BTreeMap::new();
        first.insert("b".into(), ("owner-b".into(), "b".repeat(64)));
        first.insert("a".into(), ("owner-a".into(), "a".repeat(64)));
        let mut second = BTreeMap::new();
        second.insert("a".into(), ("owner-a".into(), "a".repeat(64)));
        second.insert("b".into(), ("owner-b".into(), "b".repeat(64)));
        assert_eq!(git_inventory_digest(&first), git_inventory_digest(&second));
        second.get_mut("b").expect("row").1 = "c".repeat(64);
        assert_ne!(git_inventory_digest(&first), git_inventory_digest(&second));
    }

    #[test]
    fn authority_snapshot_rejects_restore_regression() {
        let sentinel = CutoverSentinel {
            format_version: SENTINEL_FORMAT_VERSION,
            community_id: uuid::Uuid::nil(),
            surface: "git".into(),
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
            surface: "git".into(),
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
}
