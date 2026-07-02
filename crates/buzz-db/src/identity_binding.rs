//! Corporate identity binding persistence.
//!
//! Bindings map a corporate IdP uid to the currently authorized Nostr pubkey
//! inside one Buzz community. The active uniqueness indexes deliberately model
//! one active pubkey per uid and one active uid per pubkey. Rotation/revocation
//! flows clear that active state in a follow-up lifecycle layer rather than
//! silently rewriting it during authentication.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::error::{DbError, Result};
use buzz_core::CommunityId;

/// Binding source when the IdP JWT carries the pubkey claim.
pub const SOURCE_JWT_NPUB: &str = "jwt_npub";
/// Binding source when the relay falls back to the stored uid/pubkey binding.
pub const SOURCE_DB_BINDING: &str = "db_binding";

/// Active corporate identity binding row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityBinding {
    /// Corporate IdP subject or configured stable uid claim.
    pub uid: String,
    /// Bound Nostr pubkey bytes.
    pub pubkey: Vec<u8>,
    /// Human-readable display claim captured from the latest accepted JWT.
    pub display_name: Option<String>,
    /// Source that established or last strengthened the active binding.
    pub source: String,
    /// When the binding was first created.
    pub created_at: DateTime<Utc>,
    /// When the binding row was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the binding was last seen during authentication.
    pub last_seen_at: DateTime<Utc>,
}

/// Existing active binding that conflicts with a requested binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityBindingConflict {
    /// Existing active uid.
    pub uid: String,
    /// Existing active pubkey bytes.
    pub pubkey: Vec<u8>,
    /// Existing active binding source.
    pub source: String,
}

/// Outcome of creating or validating a corporate identity binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindIdentityResult {
    /// A new active binding was created.
    Created,
    /// The requested binding matched an existing active binding.
    Matched,
    /// Another active binding already owns the uid or pubkey.
    Conflict(IdentityBindingConflict),
}

fn validate_inputs(uid: &str, pubkey: &[u8], source: &str) -> Result<()> {
    if uid.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity binding uid must not be empty".to_string(),
        ));
    }
    validate_pubkey(pubkey)?;
    if !matches!(source, SOURCE_JWT_NPUB | SOURCE_DB_BINDING) {
        return Err(DbError::InvalidData(format!(
            "invalid identity binding source: {source}"
        )));
    }
    Ok(())
}

fn validate_pubkey(pubkey: &[u8]) -> Result<()> {
    if pubkey.len() != 32 {
        return Err(DbError::InvalidData(
            "identity binding pubkey must be 32 bytes".to_string(),
        ));
    }
    Ok(())
}

fn row_to_binding(row: sqlx::postgres::PgRow) -> Result<IdentityBinding> {
    Ok(IdentityBinding {
        uid: row.try_get("uid")?,
        pubkey: row.try_get("pubkey")?,
        display_name: row.try_get("display_name")?,
        source: row.try_get("source")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
    })
}

async fn active_by_uid_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    uid: &str,
) -> Result<Option<IdentityBinding>> {
    let row = sqlx::query(
        r#"
        SELECT uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND uid = $2 AND revoked_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(row_to_binding).transpose()
}

async fn active_by_pubkey_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<IdentityBinding>> {
    let row = sqlx::query(
        r#"
        SELECT uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND pubkey = $2 AND revoked_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    row.map(row_to_binding).transpose()
}

fn conflict_from(binding: IdentityBinding) -> IdentityBindingConflict {
    IdentityBindingConflict {
        uid: binding.uid,
        pubkey: binding.pubkey,
        source: binding.source,
    }
}

async fn lock_identity_keys_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    uid: &str,
    pubkey: &[u8],
) -> Result<()> {
    let mut keys = [
        format!("{}:uid:{uid}", community_id.as_uuid()),
        format!("{}:pubkey:{}", community_id.as_uuid(), hex::encode(pubkey)),
    ];
    keys.sort();
    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('identity_bindings'), hashtext($1))")
            .bind(key)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Create or validate an active corporate identity binding.
///
/// This is a fail-closed auth-time operation:
/// - same uid + same pubkey updates display/last_seen and succeeds;
/// - same uid + different pubkey conflicts;
/// - same pubkey + different uid conflicts;
/// - no active row creates a new binding.
pub async fn bind_or_validate_identity(
    pool: &PgPool,
    community_id: CommunityId,
    uid: &str,
    pubkey: &[u8],
    display_name: Option<&str>,
    source: &str,
) -> Result<BindIdentityResult> {
    validate_inputs(uid, pubkey, source)?;

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_keys_tx(&mut tx, community_id, uid, pubkey).await?;

    let active_uid = active_by_uid_tx(&mut tx, community_id, uid).await?;
    if let Some(binding) = active_uid {
        if binding.pubkey != pubkey {
            tx.rollback().await?;
            return Ok(BindIdentityResult::Conflict(conflict_from(binding)));
        }

        sqlx::query(
            r#"
            UPDATE identity_bindings
            SET display_name = $4,
                source = CASE
                    WHEN source = 'jwt_npub' AND $5 = 'db_binding' THEN source
                    ELSE $5
                END,
                updated_at = NOW(),
                last_seen_at = NOW()
            WHERE community_id = $1 AND uid = $2 AND pubkey = $3 AND revoked_at IS NULL
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(uid)
        .bind(pubkey)
        .bind(display_name)
        .bind(source)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(BindIdentityResult::Matched);
    }

    let active_pubkey = active_by_pubkey_tx(&mut tx, community_id, pubkey).await?;
    if let Some(binding) = active_pubkey {
        if binding.uid != uid {
            tx.rollback().await?;
            return Ok(BindIdentityResult::Conflict(conflict_from(binding)));
        }
    }

    sqlx::query(
        r#"
        INSERT INTO identity_bindings (community_id, uid, pubkey, display_name, source)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(uid)
    .bind(pubkey)
    .bind(display_name)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(BindIdentityResult::Created)
}

/// Return the active binding for `pubkey`, if one exists.
pub async fn get_active_identity_binding_by_pubkey(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<IdentityBinding>> {
    validate_pubkey(pubkey)?;
    let row = sqlx::query(
        r#"
        SELECT uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND pubkey = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_binding).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("identity-binding-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    fn random_pubkey() -> Vec<u8> {
        Keys::generate().public_key().to_bytes().to_vec()
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_creates_then_matches_idempotently() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        let created = bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &pubkey,
            Some("first@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");
        assert_eq!(created, BindIdentityResult::Created);

        let matched = bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &pubkey,
            Some("second@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("match existing binding");
        assert_eq!(matched, BindIdentityResult::Matched);

        let binding = get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
            .await
            .expect("lookup binding")
            .expect("binding exists");
        assert_eq!(binding.uid, "user-1");
        assert_eq!(binding.display_name.as_deref(), Some("second@example.com"));
        assert_eq!(binding.source, SOURCE_JWT_NPUB);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_rejects_uid_conflict() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let original_pubkey = random_pubkey();
        let conflicting_pubkey = random_pubkey();

        bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &original_pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");

        let result = bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &conflicting_pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("uid conflict is a binding result");

        assert_eq!(
            result,
            BindIdentityResult::Conflict(IdentityBindingConflict {
                uid: "user-1".to_string(),
                pubkey: original_pubkey,
                source: SOURCE_DB_BINDING.to_string(),
            })
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_rejects_pubkey_conflict() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");

        let result = bind_or_validate_identity(
            &pool,
            community,
            "user-2",
            &pubkey,
            Some("other@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("pubkey conflict is a binding result");

        assert_eq!(
            result,
            BindIdentityResult::Conflict(IdentityBindingConflict {
                uid: "user-1".to_string(),
                pubkey,
                source: SOURCE_DB_BINDING.to_string(),
            })
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_does_not_downgrade_jwt_npub_source() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("create strong binding");

        let matched = bind_or_validate_identity(
            &pool,
            community,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("match existing binding");
        assert_eq!(matched, BindIdentityResult::Matched);

        let binding = get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
            .await
            .expect("lookup binding")
            .expect("binding exists");
        assert_eq!(binding.source, SOURCE_JWT_NPUB);
    }
}
