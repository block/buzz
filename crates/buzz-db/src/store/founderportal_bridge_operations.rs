//! FounderPortal bridge lifecycle operations.
//!
//! Every operation resolves Buzz coordinates from bridge mappings. Callers
//! provide FounderPortal tenant and actor coordinates only; browser-provided
//! community identifiers are never accepted as authority.

use buzz_core::CommunityId;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::{error::DbError, Result};

/// Result of an idempotent bridge operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeOperationResult {
    /// The desired state was created or changed.
    Applied,
    /// The desired state was already present.
    AlreadyApplied,
}

fn clean(value: &str, name: &str) -> Result<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed != value {
        return Err(DbError::InvalidData(format!("{name} is invalid")));
    }
    Ok(trimmed)
}

fn provisioning_host(tenant_id: &str) -> String {
    use sha2::{Digest as _, Sha256};
    format!(
        "fp-{}.founderportal.invalid",
        hex::encode(Sha256::digest(tenant_id.as_bytes()))
    )
}

/// Ensure the tenant has exactly one Buzz community.
///
/// The host is a deterministic opaque provisioning key derived solely from
/// tenant identity. It intentionally does not expose or depend on a tenant
/// name, browser host, or browser-selected community.
pub async fn ensure_community(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<(CommunityId, BridgeOperationResult)> {
    let tenant_id = clean(tenant_id, "tenant_id")?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    if let Some(row) = sqlx::query(
        "SELECT buzz_community_id, status FROM founderportal_bridge.community_mappings WHERE tenant_id = $1 FOR UPDATE",
    )
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let community_id: Option<Uuid> = row.try_get("buzz_community_id")?;
        let status: String = row.try_get("status")?;
        let community_id = community_id.ok_or_else(|| {
            DbError::InvalidData("community mapping is still provisioning".into())
        })?;
        if status != "active" {
            return Err(DbError::AccessDenied(format!(
                "community mapping is not active: {status}"
            )));
        }
        tx.commit().await?;
        return Ok((CommunityId::from_uuid(community_id), BridgeOperationResult::AlreadyApplied));
    }

    let community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community_id)
        .bind(provisioning_host(tenant_id))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO founderportal_bridge.community_mappings (tenant_id, buzz_community_id, status) VALUES ($1, $2, 'active')",
    )
    .bind(tenant_id)
    .bind(community_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((CommunityId::from_uuid(community_id), BridgeOperationResult::Applied))
}

async fn mapped_actor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
) -> Result<(CommunityId, String, String)> {
    let row = sqlx::query(
        r#"SELECT c.buzz_community_id, c.status AS community_status,
                  i.buzz_pubkey, i.status AS identity_status
           FROM founderportal_bridge.community_mappings c
           JOIN founderportal_bridge.identity_mappings i ON i.tenant_id = c.tenant_id
           WHERE c.tenant_id = $1 AND i.actor_type = $2 AND i.actor_id = $3
           FOR UPDATE OF c, i"#,
    )
    .bind(tenant_id)
    .bind(actor_type)
    .bind(actor_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::AccessDenied("trusted bridge mapping not found".into()))?;
    Ok((
        CommunityId::from_uuid(row.try_get("buzz_community_id")?),
        row.try_get("buzz_pubkey")?,
        format!(
            "{}:{}",
            row.try_get::<String, _>("community_status")?,
            row.try_get::<String, _>("identity_status")?
        ),
    ))
}

/// Ensure a mapped actor is an ordinary relay member.
pub async fn ensure_member(
    pool: &PgPool,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
) -> Result<BridgeOperationResult> {
    let tenant_id = clean(tenant_id, "tenant_id")?;
    if !matches!(actor_type, "human" | "agent") {
        return Err(DbError::InvalidData("actor_type is invalid".into()));
    }
    clean(actor_id, "actor_id")?;
    let mut tx = pool.begin().await?;
    let (community_id, pubkey, statuses) = mapped_actor(&mut tx, tenant_id, actor_type, actor_id).await?;
    if statuses != "active:active" {
        return Err(DbError::AccessDenied("bridge mapping is not active".into()));
    }
    if let Some(role) = sqlx::query_scalar::<_, String>(
        "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(community_id.as_uuid())
    .bind(&pubkey)
    .fetch_optional(&mut *tx)
    .await?
    {
        if role != "member" {
            return Err(DbError::AccessDenied(
                "mapped actor already has a privileged Buzz role".into(),
            ));
        }
        tx.commit().await?;
        return Ok(BridgeOperationResult::AlreadyApplied);
    }
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) VALUES ($1, $2, 'member', NULL) ON CONFLICT DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected() == 1;
    tx.commit().await?;
    Ok(if inserted { BridgeOperationResult::Applied } else { BridgeOperationResult::AlreadyApplied })
}

/// Remove the mapped actor's durable relay admission record.
///
/// This function implements only the durable first step. Production callers
/// must then invalidate caches, disconnect locally and cluster-wide, confirm
/// absence, and treat propagation failure as retryable.
pub async fn revoke_member_durable(
    pool: &PgPool,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
) -> Result<(CommunityId, String, BridgeOperationResult)> {
    let tenant_id = clean(tenant_id, "tenant_id")?;
    if !matches!(actor_type, "human" | "agent") {
        return Err(DbError::InvalidData("actor_type is invalid".into()));
    }
    clean(actor_id, "actor_id")?;
    let mut tx = pool.begin().await?;
    let (community_id, pubkey, _) = mapped_actor(&mut tx, tenant_id, actor_type, actor_id).await?;
    let removed = sqlx::query(
        "DELETE FROM relay_members WHERE community_id = $1 AND pubkey = $2 AND role <> 'owner'",
    )
    .bind(community_id.as_uuid())
    .bind(&pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected() == 1;
    sqlx::query(
        "UPDATE founderportal_bridge.identity_mappings SET status = 'revoked', revision = revision + 1, updated_at = now() WHERE tenant_id = $1 AND actor_type = $2 AND actor_id = $3",
    )
    .bind(tenant_id).bind(actor_type).bind(actor_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((community_id, pubkey, if removed { BridgeOperationResult::Applied } else { BridgeOperationResult::AlreadyApplied }))
}

/// Confirm the durable reconnect fence after revocation.
pub async fn confirm_member_absent(pool: &PgPool, community_id: CommunityId, pubkey: &str) -> Result<bool> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2)",
    )
    .bind(community_id.as_uuid()).bind(pubkey).fetch_one(pool).await?;
    Ok(!present)
}

/// Archive the community resolved from the trusted tenant mapping.
pub async fn archive_community(pool: &PgPool, tenant_id: &str) -> Result<(CommunityId, BridgeOperationResult)> {
    let tenant_id = clean(tenant_id, "tenant_id")?;
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT buzz_community_id, status FROM founderportal_bridge.community_mappings WHERE tenant_id = $1 FOR UPDATE",
    ).bind(tenant_id).fetch_optional(&mut *tx).await?
      .ok_or_else(|| DbError::AccessDenied("trusted bridge mapping not found".into()))?;
    let community_id: Uuid = row.try_get("buzz_community_id")?;
    let already = row.try_get::<String, _>("status")? == "archived";
    sqlx::query("UPDATE communities SET archived_at = COALESCE(archived_at, now()) WHERE id = $1")
        .bind(community_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE founderportal_bridge.community_mappings SET status = 'archived', revision = revision + 1, updated_at = now() WHERE tenant_id = $1 AND status <> 'archived'")
        .bind(tenant_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((CommunityId::from_uuid(community_id), if already { BridgeOperationResult::AlreadyApplied } else { BridgeOperationResult::Applied }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect isolated PostgreSQL test database")
    }

    fn pubkey(byte: u8) -> String { format!("{byte:02x}").repeat(32) }

    async fn bind(pool: &PgPool, tenant: &str, actor_type: &str, actor: &str, key: &str) {
        sqlx::query("INSERT INTO founderportal_bridge.identity_mappings (tenant_id,actor_type,actor_id,buzz_pubkey,status) VALUES ($1,$2,$3,$4,'active')")
            .bind(tenant).bind(actor_type).bind(actor).bind(key).execute(pool).await.expect("bind actor");
    }

    #[test]
    fn provisioning_host_is_deterministic_opaque_and_tenant_distinct() {
        let a = provisioning_host("tenant-a");
        assert_eq!(a, provisioning_host("tenant-a"));
        assert_ne!(a, provisioning_host("tenant-b"));
        assert!(!a.contains("tenant-a"));
        assert!(a.ends_with(".founderportal.invalid"));
    }

    #[tokio::test]
        async fn lifecycle_is_idempotent_atomic_and_tenant_scoped() {
        let pool = pool().await;
        let suffix = uuid::Uuid::new_v4();
        let tenant_a = format!("lifecycle-a-{suffix}");
        let tenant_b = format!("lifecycle-b-{suffix}");
        let (a1, a2) = tokio::join!(ensure_community(&pool, &tenant_a), ensure_community(&pool, &tenant_a));
        let (community_a, first) = a1.expect("first ensure");
        let (community_a_again, second) = a2.expect("second ensure");
        assert_eq!(community_a, community_a_again);
        assert_ne!(first, second);
        let (community_b, _) = ensure_community(&pool, &tenant_b).await.expect("tenant b community");
        assert_ne!(community_a, community_b);

        let shared_actor = "same-base44-user";
        let key_a = pubkey(0x31);
        let key_b = pubkey(0x32);
        bind(&pool, &tenant_a, "human", shared_actor, &key_a).await;
        bind(&pool, &tenant_b, "human", shared_actor, &key_b).await;
        assert_eq!(ensure_member(&pool, &tenant_a, "human", shared_actor).await.expect("ensure a"), BridgeOperationResult::Applied);
        assert_eq!(ensure_member(&pool, &tenant_a, "human", shared_actor).await.expect("duplicate a"), BridgeOperationResult::AlreadyApplied);
        assert_eq!(ensure_member(&pool, &tenant_b, "human", shared_actor).await.expect("ensure b"), BridgeOperationResult::Applied);

        let (revoked_community, revoked_key, result) = revoke_member_durable(&pool, &tenant_a, "human", shared_actor).await.expect("revoke a");
        assert_eq!(revoked_community, community_a);
        assert_eq!(revoked_key, key_a);
        assert_eq!(result, BridgeOperationResult::Applied);
        assert!(confirm_member_absent(&pool, community_a, &key_a).await.expect("a fence"));
        assert!(!confirm_member_absent(&pool, community_b, &key_b).await.expect("b remains"));
        assert_eq!(revoke_member_durable(&pool, &tenant_a, "human", shared_actor).await.expect("duplicate revoke").2, BridgeOperationResult::AlreadyApplied);

        assert_eq!(archive_community(&pool, &tenant_a).await.expect("archive").1, BridgeOperationResult::Applied);
        assert_eq!(archive_community(&pool, &tenant_a).await.expect("archive again").1, BridgeOperationResult::AlreadyApplied);
        let b_archived: bool = sqlx::query_scalar("SELECT archived_at IS NOT NULL FROM communities WHERE id=$1")
            .bind(community_b.as_uuid()).fetch_one(&pool).await.expect("tenant b archive state");
        assert!(!b_archived, "archiving tenant A must not archive tenant B");
    }

    #[tokio::test]
        async fn supplied_actor_coordinates_cannot_select_another_tenants_mapping() {
        let pool = pool().await;
        let suffix = uuid::Uuid::new_v4();
        let tenant_a = format!("selector-a-{suffix}");
        let tenant_b = format!("selector-b-{suffix}");
        ensure_community(&pool, &tenant_a).await.expect("community a");
        ensure_community(&pool, &tenant_b).await.expect("community b");
        bind(&pool, &tenant_a, "agent", "lookalike", &pubkey(0x41)).await;
        assert!(ensure_member(&pool, &tenant_b, "agent", "lookalike").await.is_err());
        assert!(ensure_member(&pool, &tenant_a, "human", "lookalike").await.is_err());
        assert!(ensure_member(&pool, &tenant_a, "agent", "lookalike").await.is_ok());
    }
}
