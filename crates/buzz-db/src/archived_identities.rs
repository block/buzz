//! Community-scoped archived identity persistence (NIP-IA).
//!
//! The `archived_identities` table stores a community-local UI visibility hint for
//! identity pubkeys. Archiving is not a ban: it does not affect membership,
//! relay access, or repository permissions.
//! All pubkey and event ID values are lowercase hex strings.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;

/// A single archived identity record.
#[derive(Debug, Clone)]
pub struct ArchivedIdentity {
    /// 64-char lowercase hex pubkey of the archived identity.
    pub pubkey: String,
    /// Consent path that authorized the archive: `"self"`, `"owner"`, or `"admin"`.
    pub consent_path: String,
    /// 64-char lowercase hex pubkey of the actor that requested the archive.
    pub actor: String,
    /// Optional human-readable archive reason.
    pub reason: Option<String>,
    /// Optional 64-char lowercase hex pubkey replacing this identity.
    pub replaced_by: Option<String>,
    /// Hex event ID of the archive request that created this row.
    pub request_event_id: String,
    /// When the identity was archived.
    pub archived_at: DateTime<Utc>,
}

/// Returns `true` if `pubkey` (64-char hex) is archived in `community_id`.
pub async fn is_archived(pool: &PgPool, community_id: CommunityId, pubkey: &str) -> Result<bool> {
    let row =
        sqlx::query("SELECT 1 FROM archived_identities WHERE community_id = $1 AND pubkey = $2")
            .bind(community_id.as_uuid())
            .bind(pubkey)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some())
}

/// Archives an identity in `community_id`.
///
/// Returns `true` if the row was inserted, `false` if the identity was already
/// archived in that community. Re-archiving is idempotent and does not mutate
/// the existing row.
#[allow(clippy::too_many_arguments)]
pub async fn archive(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &str,
    consent_path: &str,
    actor: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
    request_event_id: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO archived_identities \
         (community_id, pubkey, consent_path, actor, reason, replaced_by, request_event_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(consent_path)
    .bind(actor)
    .bind(reason)
    .bind(replaced_by)
    .bind(request_event_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Unarchives an identity from `community_id`.
///
/// Returns `true` if a row was deleted, `false` if the identity was not archived
/// in that community.
pub async fn unarchive(pool: &PgPool, community_id: CommunityId, pubkey: &str) -> Result<bool> {
    let result =
        sqlx::query("DELETE FROM archived_identities WHERE community_id = $1 AND pubkey = $2")
            .bind(community_id.as_uuid())
            .bind(pubkey)
            .execute(pool)
            .await?;

    Ok(result.rows_affected() > 0)
}

/// Returns all identities archived in `community_id`, ordered by archive time ascending.
pub async fn list_archived(
    pool: &PgPool,
    community_id: CommunityId,
) -> Result<Vec<ArchivedIdentity>> {
    let rows = sqlx::query(
        "SELECT pubkey, consent_path, actor, reason, replaced_by, request_event_id, archived_at \
         FROM archived_identities WHERE community_id = $1 ORDER BY archived_at ASC",
    )
    .bind(community_id.as_uuid())
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(row_to_archived_identity)
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .map_err(crate::error::DbError::from)
}

fn row_to_archived_identity(
    row: sqlx::postgres::PgRow,
) -> std::result::Result<ArchivedIdentity, sqlx::Error> {
    Ok(ArchivedIdentity {
        pubkey: row.try_get("pubkey")?,
        consent_path: row.try_get("consent_path")?,
        actor: row.try_get("actor")?,
        reason: row.try_get("reason")?,
        replaced_by: row.try_get("replaced_by")?,
        request_event_id: row.try_get("request_event_id")?,
        archived_at: row.try_get("archived_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        PgPool::connect(TEST_DB_URL)
            .await
            .expect("connect to test DB")
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = uuid::Uuid::new_v4();
        let host = format!("archive-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn archived_identity_state_is_community_scoped() {
        let pool = setup_pool().await;
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let pubkey = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let actor = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let event_a = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let event_b = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

        assert!(archive(
            &pool,
            community_a,
            pubkey,
            "self",
            actor,
            Some("community A"),
            None,
            event_a,
        )
        .await
        .expect("archive in community A"));

        assert!(is_archived(&pool, community_a, pubkey)
            .await
            .expect("is_archived in A"));
        assert!(!is_archived(&pool, community_b, pubkey)
            .await
            .expect("is_archived in B"));
        assert_eq!(
            list_archived(&pool, community_a)
                .await
                .expect("list A")
                .len(),
            1
        );
        assert!(list_archived(&pool, community_b)
            .await
            .expect("list B")
            .is_empty());
        assert!(!unarchive(&pool, community_b, pubkey)
            .await
            .expect("unarchive absent B"));
        assert!(is_archived(&pool, community_a, pubkey)
            .await
            .expect("B unarchive must not affect A"));

        assert!(archive(
            &pool,
            community_b,
            pubkey,
            "self",
            actor,
            Some("community B"),
            None,
            event_b,
        )
        .await
        .expect("archive same pubkey in community B"));
        assert!(unarchive(&pool, community_a, pubkey)
            .await
            .expect("unarchive A"));
        assert!(!is_archived(&pool, community_a, pubkey)
            .await
            .expect("A removed"));
        assert!(is_archived(&pool, community_b, pubkey)
            .await
            .expect("A unarchive must not affect B"));
    }
}
