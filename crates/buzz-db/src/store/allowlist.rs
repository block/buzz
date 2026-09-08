//! Community-scoped authentication allowlist persistence.
//!
//! This store is distinct from NIP-43 relay membership. Membership backfill
//! orchestration remains with the relay-membership invariant owner.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::error::Result;
use crate::Db;

/// An entry in the pubkey allowlist.
#[derive(Debug, Clone)]
pub struct AllowlistEntry {
    /// The allowed pubkey.
    pub pubkey: Vec<u8>,
    /// Who added this entry.
    pub added_by: Vec<u8>,
    /// When the entry was added.
    pub added_at: DateTime<Utc>,
    /// Optional note.
    pub note: Option<String>,
}

impl Db {
    /// Check if a pubkey is in the allowlist for `community`.
    #[datastore_span(name = "is_pubkey_allowed", system = "postgresql")]
    pub async fn is_pubkey_allowed(&self, community: CommunityId, pubkey: &[u8]) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authentication,
        )
        .await?;
        let row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM pubkey_allowlist WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .fetch_one(&mut *connection)
        .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Check if the community allowlist has any entries (i.e. is enforcement active).
    #[datastore_span(name = "has_allowlist_entries", system = "postgresql")]
    pub async fn has_allowlist_entries(&self, community: CommunityId) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authentication,
        )
        .await?;
        let row =
            sqlx::query("SELECT COUNT(*) as cnt FROM pubkey_allowlist WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&mut *connection)
                .await?;
        let cnt: i64 = row.try_get("cnt")?;
        Ok(cnt > 0)
    }

    /// Add a pubkey to the community allowlist.
    #[datastore_span(name = "add_to_allowlist", system = "postgresql")]
    pub async fn add_to_allowlist(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        added_by: &[u8],
        note: Option<&str>,
    ) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let result = sqlx::query(
            "INSERT INTO pubkey_allowlist (community_id, pubkey, added_by, note) VALUES ($1, $2, $3, $4) \
             ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(pubkey)
        .bind(added_by)
        .bind(note)
        .execute(&mut *connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove a pubkey from the community allowlist.
    #[datastore_span(name = "remove_from_allowlist", system = "postgresql")]
    pub async fn remove_from_allowlist(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let result =
            sqlx::query("DELETE FROM pubkey_allowlist WHERE community_id = $1 AND pubkey = $2")
                .bind(community.as_uuid())
                .bind(pubkey)
                .execute(&mut *connection)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    /// List all pubkeys in the community allowlist.
    #[datastore_span(name = "list_allowlist", system = "postgresql")]
    pub async fn list_allowlist(&self, community: CommunityId) -> Result<Vec<AllowlistEntry>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT pubkey, added_by, added_at, note FROM pubkey_allowlist WHERE community_id = $1 ORDER BY added_at DESC",
        )
        .bind(community.as_uuid())
        .fetch_all(&mut *connection)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(AllowlistEntry {
                pubkey: row.try_get("pubkey")?,
                added_by: row.try_get("added_by")?,
                added_at: row.try_get("added_at")?,
                note: row.try_get("note")?,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

    async fn setup_db() -> Db {
        let database_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("communities-of-channels-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn allowlist_is_scoped_to_community() {
        let db = setup_db().await;
        let community_a = CommunityId::from_uuid(make_community(&db.pool).await);
        let community_b = CommunityId::from_uuid(make_community(&db.pool).await);
        let pubkey = [7u8; 32];
        let added_by = [9u8; 32];

        assert!(db
            .add_to_allowlist(community_a, &pubkey, &added_by, Some("a-only"))
            .await
            .expect("add allowlist row"));
        assert!(!db
            .add_to_allowlist(community_a, &pubkey, &added_by, Some("duplicate"))
            .await
            .expect("duplicate allowlist row is idempotent"));

        assert!(
            db.is_pubkey_allowed(community_a, &pubkey)
                .await
                .expect("allowlist check A"),
            "pubkey added to A must be allowed in A"
        );
        assert!(
            !db.is_pubkey_allowed(community_b, &pubkey)
                .await
                .expect("allowlist check B"),
            "pubkey added only to A must not be allowed in B"
        );
        assert!(db
            .has_allowlist_entries(community_a)
            .await
            .expect("A has entries"));
        assert!(!db
            .has_allowlist_entries(community_b)
            .await
            .expect("B has no entries"));

        let listed = db
            .list_allowlist(community_a)
            .await
            .expect("list A allowlist");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pubkey, pubkey);

        assert!(
            !db.remove_from_allowlist(community_b, &pubkey)
                .await
                .expect("remove from B is no-op"),
            "removing from B must not delete A's row"
        );
        assert!(db
            .is_pubkey_allowed(community_a, &pubkey)
            .await
            .expect("A still allowed after B remove"));
        assert!(db
            .remove_from_allowlist(community_a, &pubkey)
            .await
            .expect("remove from A"));
        assert!(!db
            .is_pubkey_allowed(community_a, &pubkey)
            .await
            .expect("A not allowed after remove"));
    }
}
