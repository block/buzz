//! Monotonic migration authority for protected Git and media visibility.

use buzz_core::CommunityId;
use sqlx::{Postgres, Row, Transaction};

use crate::{Db, DbError, Result};

/// Object-store visibility family being migrated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedObjectSurface {
    /// Git repository pointers and manifests.
    Git,
    /// Media sidecars and immutable blobs.
    Media,
}

impl ProtectedObjectSurface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Media => "media",
        }
    }
}

/// Durable authority state for one community and surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedObjectAuthorityState {
    /// Legacy pointer or sidecar remains authoritative.
    Legacy,
    /// Legacy writes are fenced while an idempotent import is validated.
    Importing,
    /// PostgreSQL is the sole visibility authority.
    PostgreSql,
}

/// Current durable migration state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedObjectAuthority {
    /// Monotonic migration generation.
    pub generation: u64,
    /// Current authority state.
    pub state: ProtectedObjectAuthorityState,
    /// Exact imported-object count recorded at cutover.
    pub imported_objects: Option<u64>,
    /// Exact imported inventory digest recorded at cutover.
    pub inventory_sha256: Option<String>,
}

fn decode_state(state: &str) -> Result<ProtectedObjectAuthorityState> {
    match state {
        "legacy" => Ok(ProtectedObjectAuthorityState::Legacy),
        "importing" => Ok(ProtectedObjectAuthorityState::Importing),
        "postgresql" => Ok(ProtectedObjectAuthorityState::PostgreSql),
        _ => Err(DbError::InvalidData(
            "protected object authority state is invalid".into(),
        )),
    }
}

fn decode_generation(value: i64) -> Result<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            DbError::InvalidData("protected object authority generation is invalid".into())
        })
}

async fn ensure_row(
    transaction: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    surface: ProtectedObjectSurface,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO protected_object_authority \
         (community_id, surface, state, generation) VALUES ($1, $2, 'legacy', 1) \
         ON CONFLICT (community_id, surface) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(surface.as_str())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

impl Db {
    /// Read durable authority, treating an untouched domain as legacy.
    pub async fn protected_object_authority(
        &self,
        community: CommunityId,
        surface: ProtectedObjectSurface,
    ) -> Result<ProtectedObjectAuthority> {
        let row = sqlx::query(
            "SELECT state, generation, imported_objects, inventory_sha256 \
             FROM protected_object_authority \
             WHERE community_id = $1 AND surface = $2",
        )
        .bind(community.as_uuid())
        .bind(surface.as_str())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => {
                let imported_objects: i64 = row.try_get("imported_objects")?;
                Ok(ProtectedObjectAuthority {
                    state: decode_state(row.try_get("state")?)?,
                    generation: decode_generation(row.try_get("generation")?)?,
                    imported_objects: (imported_objects > 0)
                        .then(|| u64::try_from(imported_objects).ok())
                        .flatten()
                        .or_else(|| (imported_objects == 0).then_some(0)),
                    inventory_sha256: row.try_get("inventory_sha256")?,
                })
            }
            None => Ok(ProtectedObjectAuthority {
                state: ProtectedObjectAuthorityState::Legacy,
                generation: 1,
                imported_objects: None,
                inventory_sha256: None,
            }),
        }
    }

    /// Begin or resume an import after draining transaction-held legacy writers.
    pub async fn begin_protected_object_import(
        &self,
        community: CommunityId,
        surface: ProtectedObjectSurface,
    ) -> Result<ProtectedObjectAuthority> {
        let mut transaction = self.begin_transaction().await?;
        ensure_row(&mut transaction, community, surface).await?;
        let row = sqlx::query(
            "SELECT state, generation FROM protected_object_authority \
             WHERE community_id = $1 AND surface = $2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(surface.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        let state = decode_state(row.try_get("state")?)?;
        let mut generation = decode_generation(row.try_get("generation")?)?;
        if state == ProtectedObjectAuthorityState::Legacy {
            generation = generation.checked_add(1).ok_or_else(|| {
                DbError::InvalidData("protected object authority generation exhausted".into())
            })?;
            let generation_i64 = i64::try_from(generation).map_err(|_| {
                DbError::InvalidData("protected object authority generation exhausted".into())
            })?;
            sqlx::query(
                "UPDATE protected_object_authority SET state = 'importing', \
                 generation = $3, started_at = clock_timestamp(), completed_at = NULL, \
                 imported_objects = 0, inventory_sha256 = NULL, \
                 updated_at = clock_timestamp() \
                 WHERE community_id = $1 AND surface = $2",
            )
            .bind(community.as_uuid())
            .bind(surface.as_str())
            .bind(generation_i64)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(ProtectedObjectAuthority {
            generation,
            state: if state == ProtectedObjectAuthorityState::Legacy {
                ProtectedObjectAuthorityState::Importing
            } else {
                state
            },
            imported_objects: None,
            inventory_sha256: None,
        })
    }

    /// Finalize one exact import generation after a complete parity pass.
    pub async fn finalize_protected_object_import(
        &self,
        community: CommunityId,
        surface: ProtectedObjectSurface,
        generation: u64,
        imported_objects: u64,
        inventory_sha256: &str,
    ) -> Result<()> {
        validate_inventory_digest(inventory_sha256)?;
        let generation = i64::try_from(generation).map_err(|_| {
            DbError::InvalidData("protected object authority generation is invalid".into())
        })?;
        let imported_objects = i64::try_from(imported_objects)
            .map_err(|_| DbError::InvalidData("protected object import count is invalid".into()))?;
        let result = sqlx::query(
            "UPDATE protected_object_authority SET state = 'postgresql', \
             imported_objects = $4, inventory_sha256 = $5, \
             completed_at = clock_timestamp(), updated_at = clock_timestamp() \
             WHERE community_id = $1 AND surface = $2 AND state = 'importing' \
               AND generation = $3",
        )
        .bind(community.as_uuid())
        .bind(surface.as_str())
        .bind(generation)
        .bind(imported_objects)
        .bind(inventory_sha256)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            let current = self.protected_object_authority(community, surface).await?;
            if current.state != ProtectedObjectAuthorityState::PostgreSql
                || current.generation != u64::try_from(generation).unwrap_or_default()
                || current.imported_objects
                    != Some(u64::try_from(imported_objects).unwrap_or_default())
                || current.inventory_sha256.as_deref() != Some(inventory_sha256)
            {
                return Err(DbError::InvalidData(
                    "protected object import generation changed".into(),
                ));
            }
        }
        Ok(())
    }

    /// Acquire a transaction-held fence spanning a legacy pointer/sidecar write.
    pub async fn begin_legacy_visibility_write(
        &self,
        community: CommunityId,
        surface: ProtectedObjectSurface,
    ) -> Result<LegacyVisibilityWrite> {
        let mut transaction = self.begin_transaction().await?;
        ensure_row(&mut transaction, community, surface).await?;
        let state: String = sqlx::query_scalar(
            "SELECT state FROM protected_object_authority \
             WHERE community_id = $1 AND surface = $2 FOR SHARE",
        )
        .bind(community.as_uuid())
        .bind(surface.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        if decode_state(&state)? != ProtectedObjectAuthorityState::Legacy {
            return Err(DbError::InvalidData(
                "legacy object visibility is no longer writable".into(),
            ));
        }
        Ok(LegacyVisibilityWrite { transaction })
    }
}

/// Transaction lock held across one actual legacy visibility write.
pub struct LegacyVisibilityWrite {
    transaction: Transaction<'static, Postgres>,
}

impl LegacyVisibilityWrite {
    /// Commit after the pointer or sidecar has become visible.
    pub async fn commit(self) -> Result<()> {
        self.transaction.commit().await.map_err(Into::into)
    }
}

/// Require an exact authority state inside a caller-owned transaction.
pub async fn require_protected_object_authority(
    transaction: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    surface: ProtectedObjectSurface,
    expected: ProtectedObjectAuthorityState,
) -> Result<()> {
    ensure_row(transaction, community, surface).await?;
    let state: String = sqlx::query_scalar(
        "SELECT state FROM protected_object_authority \
         WHERE community_id = $1 AND surface = $2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(surface.as_str())
    .fetch_one(&mut **transaction)
    .await?;
    if decode_state(&state)? != expected {
        return Err(DbError::InvalidData(
            "protected object visibility authority is unavailable".into(),
        ));
    }
    Ok(())
}

fn validate_inventory_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
    {
        return Err(DbError::InvalidData(
            "protected object inventory digest is invalid".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use uuid::Uuid;

    #[test]
    fn authority_state_decoder_is_closed() {
        assert_eq!(
            decode_state("postgresql").expect("known state"),
            ProtectedObjectAuthorityState::PostgreSql
        );
        assert!(decode_state("fallback").is_err());
    }

    #[test]
    fn inventory_digest_is_strict_lowercase_sha256() {
        assert!(validate_inventory_digest(&"a".repeat(64)).is_ok());
        assert!(validate_inventory_digest(&"A".repeat(64)).is_err());
        assert!(validate_inventory_digest("short").is_err());
    }

    #[tokio::test]
    #[ignore = "requires migrated Postgres"]
    async fn cutover_waits_for_legacy_commit_and_never_reverses() {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_owned());
        let pool = PgPool::connect(&database_url).await.expect("test database");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("migrated test database");
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("visibility-{}.example", id.simple()))
            .execute(&pool)
            .await
            .expect("community");
        let db = Db::from_pool(pool);
        let community = CommunityId::from_uuid(id);

        let guard = db
            .begin_legacy_visibility_write(community, ProtectedObjectSurface::Git)
            .await
            .expect("legacy writer");
        let contender_pool = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET lock_timeout = '100ms'")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await
            .expect("contender database");
        let contender = Db::from_pool(contender_pool);
        let blocked = contender
            .begin_protected_object_import(community, ProtectedObjectSurface::Git)
            .await
            .expect_err("cutover must wait for the legacy writer");
        assert!(
            matches!(
                blocked,
                DbError::Sqlx(sqlx::Error::Database(ref error))
                    if error.code().as_deref() == Some("55P03")
            ),
            "the real cutover row lock must block: {blocked:?}"
        );
        guard.commit().await.expect("legacy commit");
        let importing = db
            .begin_protected_object_import(community, ProtectedObjectSurface::Git)
            .await
            .expect("begin import");
        assert_eq!(importing.state, ProtectedObjectAuthorityState::Importing);
        assert!(db
            .begin_legacy_visibility_write(community, ProtectedObjectSurface::Git)
            .await
            .is_err());

        db.finalize_protected_object_import(
            community,
            ProtectedObjectSurface::Git,
            importing.generation,
            0,
            &"0".repeat(64),
        )
        .await
        .expect("finalize");
        let finished = db
            .begin_protected_object_import(community, ProtectedObjectSurface::Git)
            .await
            .expect("monotonic retry");
        assert_eq!(finished.state, ProtectedObjectAuthorityState::PostgreSql);
        assert_eq!(finished.generation, importing.generation);
    }
}
