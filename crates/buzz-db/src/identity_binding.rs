//! Corporate identity binding persistence.
//!
//! Bindings map an issuer-qualified IdP uid to the currently authorized Nostr
//! pubkey inside one Buzz community. The active uniqueness indexes deliberately
//! model one active pubkey per `(issuer, uid)` principal and one active principal
//! per pubkey. Explicit lifecycle operations distinguish principal disablement,
//! single-key revocation, and authorized rotation; authentication never
//! silently rewrites those states.

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
    /// Validated identity-provider issuer.
    pub issuer: String,
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
    /// Existing active issuer.
    pub issuer: String,
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
    /// The requested uid/pubkey pair was previously revoked.
    Revoked,
}

/// Corporate identity data staged for an atomic admission transaction.
#[derive(Debug, Clone, Copy)]
pub struct IdentityBindingInput<'a> {
    /// Validated identity-provider issuer.
    pub issuer: &'a str,
    /// Stable issuer-qualified principal identifier.
    pub uid: &'a str,
    /// Authenticated Nostr pubkey bytes.
    pub pubkey: &'a [u8],
    /// Private display attribute retained in the binding table.
    pub display_name: Option<&'a str>,
    /// Binding source (`jwt_npub` or `db_binding`).
    pub source: &'a str,
}

fn validate_inputs(issuer: &str, uid: &str, pubkey: &[u8], source: &str) -> Result<()> {
    if issuer.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity binding issuer must not be empty".to_string(),
        ));
    }
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

pub(crate) fn validate_membership_identity_key(
    member_pubkey_hex: &str,
    identity: Option<&IdentityBindingInput<'_>>,
) -> Result<()> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let member_pubkey = hex::decode(member_pubkey_hex)
        .map_err(|_| DbError::InvalidData("membership pubkey must be 32-byte hex".to_string()))?;
    if member_pubkey.len() != 32 || member_pubkey.as_slice() != identity.pubkey {
        return Err(DbError::InvalidData(
            "membership pubkey does not match staged identity key".to_string(),
        ));
    }
    Ok(())
}

fn row_to_binding(row: sqlx::postgres::PgRow) -> Result<IdentityBinding> {
    Ok(IdentityBinding {
        issuer: row.try_get("issuer")?,
        uid: row.try_get("uid")?,
        pubkey: row.try_get("pubkey")?,
        display_name: row.try_get("display_name")?,
        source: row.try_get("source")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
    })
}

async fn active_by_principal_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<Option<IdentityBinding>> {
    let row = sqlx::query(
        r#"
        SELECT issuer, uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
        FROM identity_bindings
        WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND revoked_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
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
        SELECT issuer, uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
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

async fn revoked_pair_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    pubkey: &[u8],
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1
        FROM identity_bindings
        WHERE community_id = $1
          AND issuer = $2
          AND uid = $3
          AND pubkey = $4
          AND revoked_at IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn principal_disabled_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1 FROM identity_principals
        WHERE community_id = $1 AND issuer = $2 AND uid = $3
          AND disabled_at IS NOT NULL
        UNION ALL
        SELECT 1 FROM identity_bindings
        WHERE community_id = $1 AND issuer = $2 AND uid = $3
          AND revoked_at IS NOT NULL AND revocation_scope = 'principal'
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn key_revoked_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1 FROM identity_revoked_keys
        WHERE community_id = $1 AND pubkey = $2
        UNION ALL
        SELECT 1 FROM identity_bindings
        WHERE community_id = $1 AND pubkey = $2 AND revoked_at IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

async fn principal_requires_rotation_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
) -> Result<bool> {
    let row = sqlx::query(
        r#"
        SELECT 1
        FROM identity_bindings
        WHERE community_id = $1
          AND issuer = $2
          AND uid = $3
          AND revoked_at IS NOT NULL
          AND revocation_scope = 'key'
          AND rotation_completed_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

fn conflict_from(binding: IdentityBinding) -> IdentityBindingConflict {
    IdentityBindingConflict {
        issuer: binding.issuer,
        uid: binding.uid,
        pubkey: binding.pubkey,
        source: binding.source,
    }
}

async fn lock_identity_key_strings_tx(
    tx: &mut Transaction<'_, Postgres>,
    mut keys: Vec<String>,
) -> Result<()> {
    keys.sort();
    keys.dedup();
    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('identity_bindings'), hashtext($1))")
            .bind(key)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn lock_identity_keys_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    pubkey: &[u8],
) -> Result<()> {
    lock_identity_key_strings_tx(
        tx,
        vec![
            format!("{}:principal:{issuer}:{uid}", community_id.as_uuid()),
            format!("{}:pubkey:{}", community_id.as_uuid(), hex::encode(pubkey)),
        ],
    )
    .await
}

/// Create or validate an active corporate identity binding.
///
/// This is a fail-closed auth-time operation:
/// - same issuer + uid + pubkey updates display/last_seen and succeeds;
/// - same issuer + uid with a different pubkey conflicts;
/// - same pubkey with a different issuer-qualified principal conflicts;
/// - principal disablement and unresolved key revocation reject every key;
/// - a previously revoked issuer/uid/pubkey tuple remains revoked;
/// - no active row creates a new binding.
pub async fn bind_or_validate_identity(
    pool: &PgPool,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    pubkey: &[u8],
    display_name: Option<&str>,
    source: &str,
) -> Result<BindIdentityResult> {
    let mut tx = pool.begin().await?;
    let result = bind_or_validate_identity_tx(
        &mut tx,
        community_id,
        &IdentityBindingInput {
            issuer,
            uid,
            pubkey,
            display_name,
            source,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// Create or validate a binding inside a caller-owned admission transaction.
pub(crate) async fn bind_or_validate_identity_tx(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    identity: &IdentityBindingInput<'_>,
) -> Result<BindIdentityResult> {
    let IdentityBindingInput {
        issuer,
        uid,
        pubkey,
        display_name,
        source,
    } = *identity;
    validate_inputs(issuer, uid, pubkey, source)?;

    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut **tx)
        .await?;
    lock_identity_keys_tx(tx, community_id, issuer, uid, pubkey).await?;

    if principal_disabled_tx(tx, community_id, issuer, uid).await? {
        return Ok(BindIdentityResult::Revoked);
    }
    if key_revoked_tx(tx, community_id, pubkey).await? {
        return Ok(BindIdentityResult::Revoked);
    }
    if principal_requires_rotation_tx(tx, community_id, issuer, uid).await? {
        return Ok(BindIdentityResult::Revoked);
    }

    let active_principal = active_by_principal_tx(tx, community_id, issuer, uid).await?;
    if let Some(binding) = active_principal {
        if binding.pubkey != pubkey {
            return Ok(BindIdentityResult::Conflict(conflict_from(binding)));
        }

        sqlx::query(
            r#"
            UPDATE identity_bindings
            SET display_name = $5,
                source = CASE
                    WHEN source = 'jwt_npub' AND $6 = 'db_binding' THEN source
                    ELSE $6
                END,
                updated_at = NOW(),
                last_seen_at = NOW()
            WHERE community_id = $1
              AND issuer = $2
              AND uid = $3
              AND pubkey = $4
              AND revoked_at IS NULL
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(issuer)
        .bind(uid)
        .bind(pubkey)
        .bind(display_name)
        .bind(source)
        .execute(&mut **tx)
        .await?;
        return Ok(BindIdentityResult::Matched);
    }

    let active_pubkey = active_by_pubkey_tx(tx, community_id, pubkey).await?;
    if let Some(binding) = active_pubkey {
        if binding.issuer != issuer || binding.uid != uid {
            return Ok(BindIdentityResult::Conflict(conflict_from(binding)));
        }
    }

    if revoked_pair_exists_tx(tx, community_id, issuer, uid, pubkey).await? {
        return Ok(BindIdentityResult::Revoked);
    }

    sqlx::query(
        r#"
        INSERT INTO identity_bindings (community_id, issuer, uid, pubkey, display_name, source)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(pubkey)
    .bind(display_name)
    .bind(source)
    .execute(&mut **tx)
    .await?;
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
        SELECT issuer, uid, pubkey, display_name, source, created_at, updated_at, last_seen_at
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

/// Disable an issuer-qualified principal and revoke its active key.
///
/// A principal disablement is durable: normal authentication with any new key
/// returns [`BindIdentityResult::Revoked`]. Re-enablement requires a separate,
/// explicit operator lifecycle operation rather than first-use enrollment.
pub async fn revoke_identity_principal(
    pool: &PgPool,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    revoked_by: Option<&[u8]>,
    reason: &str,
) -> Result<bool> {
    if issuer.trim().is_empty() || uid.trim().is_empty() || reason.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity principal revocation requires issuer, uid, and reason".to_string(),
        ));
    }
    if let Some(pubkey) = revoked_by {
        validate_pubkey(pubkey)?;
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    // Enrollment and rotation take the principal lock first. Take it before
    // discovering the current key so a concurrent first enrollment cannot
    // slip between the lookup and the durable principal tombstone.
    lock_identity_key_strings_tx(
        &mut tx,
        vec![format!(
            "{}:principal:{issuer}:{uid}",
            community_id.as_uuid()
        )],
    )
    .await?;
    let active_pubkey: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT pubkey FROM identity_bindings WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND revoked_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(pubkey) = active_pubkey.as_ref() {
        lock_identity_key_strings_tx(
            &mut tx,
            vec![format!(
                "{}:pubkey:{}",
                community_id.as_uuid(),
                hex::encode(pubkey)
            )],
        )
        .await?;
    }

    sqlx::query(
        r#"
        INSERT INTO identity_principals
            (community_id, issuer, uid, disabled_at, disabled_by, disabled_reason)
        VALUES ($1, $2, $3, NOW(), $4, $5)
        ON CONFLICT (community_id, issuer, uid) DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(revoked_by)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE identity_bindings
        SET revoked_at = NOW(), revoked_by = $4, revoked_reason = $5,
            revocation_scope = 'principal', updated_at = NOW()
        WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND revoked_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(revoked_by)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Revoke one active key without disabling the issuer-qualified principal.
/// A replacement key must still be installed through [`rotate_identity_binding`].
pub async fn revoke_identity_key(
    pool: &PgPool,
    community_id: CommunityId,
    pubkey: &[u8],
    revoked_by: Option<&[u8]>,
    reason: &str,
) -> Result<bool> {
    validate_pubkey(pubkey)?;
    if let Some(operator) = revoked_by {
        validate_pubkey(operator)?;
    }
    if reason.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity key revocation reason must not be empty".to_string(),
        ));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    // A community-scoped key tombstone is the entire correctness boundary.
    // Do not acquire a principal lock after it: enrollment and rotation use
    // principal→key ordering, and reversing that order can deadlock.
    lock_identity_key_strings_tx(
        &mut tx,
        vec![format!(
            "{}:pubkey:{}",
            community_id.as_uuid(),
            hex::encode(pubkey)
        )],
    )
    .await?;
    sqlx::query(
        r#"
        INSERT INTO identity_revoked_keys
            (community_id, pubkey, revoked_at, revoked_by, reason)
        VALUES ($1, $2, NOW(), $3, $4)
        ON CONFLICT (community_id, pubkey) DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(revoked_by)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE identity_bindings
        SET revoked_at = NOW(), revoked_by = $3, revoked_reason = $4,
            revocation_scope = 'key', updated_at = NOW()
        WHERE community_id = $1 AND pubkey = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(revoked_by)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Atomically retire an active key and install an operator-authorized replacement.
#[allow(clippy::too_many_arguments)]
pub async fn rotate_identity_binding(
    pool: &PgPool,
    community_id: CommunityId,
    issuer: &str,
    uid: &str,
    old_pubkey: &[u8],
    new_pubkey: &[u8],
    display_name: Option<&str>,
    source: &str,
    rotated_by: Option<&[u8]>,
    reason: &str,
) -> Result<()> {
    validate_inputs(issuer, uid, old_pubkey, source)?;
    validate_pubkey(new_pubkey)?;
    if old_pubkey == new_pubkey {
        return Err(DbError::InvalidData(
            "identity rotation requires a different replacement key".to_string(),
        ));
    }
    if let Some(operator) = rotated_by {
        validate_pubkey(operator)?;
    }
    if reason.trim().is_empty() {
        return Err(DbError::InvalidData(
            "identity rotation reason must not be empty".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *tx)
        .await?;
    lock_identity_key_strings_tx(
        &mut tx,
        vec![
            format!("{}:principal:{issuer}:{uid}", community_id.as_uuid()),
            format!(
                "{}:pubkey:{}",
                community_id.as_uuid(),
                hex::encode(old_pubkey)
            ),
            format!(
                "{}:pubkey:{}",
                community_id.as_uuid(),
                hex::encode(new_pubkey)
            ),
        ],
    )
    .await?;
    if principal_disabled_tx(&mut tx, community_id, issuer, uid).await? {
        return Err(DbError::InvalidData(
            "disabled identity principal cannot be rotated".to_string(),
        ));
    }
    if key_revoked_tx(&mut tx, community_id, new_pubkey).await? {
        return Err(DbError::InvalidData(
            "identity rotation replacement key is revoked".to_string(),
        ));
    }
    let active = active_by_principal_tx(&mut tx, community_id, issuer, uid).await?;
    if active
        .as_ref()
        .is_some_and(|binding| binding.pubkey != old_pubkey)
    {
        return Err(DbError::InvalidData(
            "identity rotation source key does not match active binding".to_string(),
        ));
    }
    if active.is_none() {
        let revoked_key = sqlx::query(
            r#"
            SELECT 1 FROM identity_bindings
            WHERE community_id = $1 AND issuer = $2 AND uid = $3
              AND pubkey = $4 AND revoked_at IS NOT NULL
              AND revocation_scope = 'key'
            FOR UPDATE
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(issuer)
        .bind(uid)
        .bind(old_pubkey)
        .fetch_optional(&mut *tx)
        .await?;
        if revoked_key.is_none() {
            return Err(DbError::InvalidData(
                "identity rotation source is neither active nor key-revoked".to_string(),
            ));
        }
    }
    if active_by_pubkey_tx(&mut tx, community_id, new_pubkey)
        .await?
        .is_some()
    {
        return Err(DbError::InvalidData(
            "identity rotation replacement key is already bound".to_string(),
        ));
    }

    sqlx::query(
        r#"
        UPDATE identity_bindings
        SET revoked_at = COALESCE(revoked_at, NOW()),
            revoked_by = CASE WHEN revoked_at IS NULL THEN $5 ELSE revoked_by END,
            revoked_reason = CASE WHEN revoked_at IS NULL THEN $6 ELSE revoked_reason END,
            revocation_scope = CASE WHEN revoked_at IS NULL THEN 'rotation' ELSE revocation_scope END,
            rotation_completed_at = NOW(), rotated_to_pubkey = $7,
            rotation_by = $5, rotation_reason = $6, updated_at = NOW()
        WHERE community_id = $1 AND issuer = $2 AND uid = $3
          AND pubkey = $4
          AND (revoked_at IS NULL OR revocation_scope = 'key')
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(old_pubkey)
    .bind(rotated_by)
    .bind(reason)
    .bind(new_pubkey)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO identity_revoked_keys
            (community_id, pubkey, revoked_at, revoked_by, reason)
        VALUES ($1, $2, NOW(), $3, $4)
        ON CONFLICT (community_id, pubkey) DO NOTHING
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(old_pubkey)
    .bind(rotated_by)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO identity_bindings
            (community_id, issuer, uid, pubkey, display_name, source)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(issuer)
    .bind(uid)
    .bind(new_pubkey)
    .bind(display_name)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";
    const TEST_ISSUER: &str = "https://idp.example";

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

    #[test]
    fn staged_identity_key_must_match_membership_key() {
        let identity_key = [7_u8; 32];
        let other_key = [8_u8; 32];
        let identity = IdentityBindingInput {
            issuer: TEST_ISSUER,
            uid: "user-1",
            pubkey: &identity_key,
            display_name: None,
            source: SOURCE_JWT_NPUB,
        };

        validate_membership_identity_key(&hex::encode(identity_key), Some(&identity))
            .expect("matching key");
        assert!(
            validate_membership_identity_key(&hex::encode(other_key), Some(&identity)).is_err()
        );
        assert!(validate_membership_identity_key("not-hex", Some(&identity)).is_err());
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
            TEST_ISSUER,
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
            TEST_ISSUER,
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
        assert_eq!(binding.issuer, TEST_ISSUER);
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
            TEST_ISSUER,
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
            TEST_ISSUER,
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
                issuer: TEST_ISSUER.to_string(),
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
            TEST_ISSUER,
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
            TEST_ISSUER,
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
                issuer: TEST_ISSUER.to_string(),
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
            TEST_ISSUER,
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
            TEST_ISSUER,
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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_does_not_recreate_revoked_pair() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();

        bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("create binding");

        sqlx::query(
            r#"
            UPDATE identity_bindings
            SET revoked_at = NOW(), revoked_reason = 'test revocation'
            WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4
            "#,
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("user-1")
        .bind(&pubkey)
        .execute(&pool)
        .await
        .expect("revoke binding");

        let result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("revoked pair is a binding result");

        assert_eq!(result, BindIdentityResult::Revoked);
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &pubkey)
                .await
                .expect("lookup binding")
                .is_none()
        );

        let replacement = random_pubkey();
        let replacement_result = bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "user-1",
            &replacement,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("principal revocation is a binding result");
        assert_eq!(replacement_result, BindIdentityResult::Revoked);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn authorized_rotation_retires_old_key_and_installs_replacement() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let old_pubkey = random_pubkey();
        let new_pubkey = random_pubkey();
        bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "rotating-user",
            &old_pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
        )
        .await
        .expect("create binding");

        rotate_identity_binding(
            &pool,
            community,
            TEST_ISSUER,
            "rotating-user",
            &old_pubkey,
            &new_pubkey,
            Some("user@example.com"),
            SOURCE_JWT_NPUB,
            None,
            "device replacement",
        )
        .await
        .expect("authorized rotation");

        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &old_pubkey)
                .await
                .expect("old lookup")
                .is_none()
        );
        assert_eq!(
            get_active_identity_binding_by_pubkey(&pool, community, &new_pubkey)
                .await
                .expect("new lookup")
                .expect("replacement active")
                .uid,
            "rotating-user"
        );
        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "rotating-user",
                &old_pubkey,
                Some("user@example.com"),
                SOURCE_JWT_NPUB,
            )
            .await
            .expect("old key result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn key_revocation_requires_explicit_rotation_for_replacement() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let old_pubkey = random_pubkey();
        let new_pubkey = random_pubkey();
        bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "key-revoked-user",
            &old_pubkey,
            None,
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");
        assert!(
            revoke_identity_key(&pool, community, &old_pubkey, None, "lost device",)
                .await
                .expect("revoke key")
        );

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "key-revoked-user",
                &new_pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("automatic replacement result"),
            BindIdentityResult::Revoked
        );

        rotate_identity_binding(
            &pool,
            community,
            TEST_ISSUER,
            "key-revoked-user",
            &old_pubkey,
            &new_pubkey,
            None,
            SOURCE_DB_BINDING,
            None,
            "approved replacement",
        )
        .await
        .expect("explicit rotation after key revocation");
        assert!(
            get_active_identity_binding_by_pubkey(&pool, community, &new_pubkey)
                .await
                .expect("replacement lookup")
                .is_some()
        );
        let retired = sqlx::query(
            "SELECT revoked_reason, revocation_scope, rotation_reason, rotated_to_pubkey \
             FROM identity_bindings \
             WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("key-revoked-user")
        .bind(&old_pubkey)
        .fetch_one(&pool)
        .await
        .expect("retired binding provenance");
        assert_eq!(
            retired.try_get::<String, _>("revoked_reason").unwrap(),
            "lost device"
        );
        assert_eq!(
            retired.try_get::<String, _>("revocation_scope").unwrap(),
            "key"
        );
        assert_eq!(
            retired.try_get::<String, _>("rotation_reason").unwrap(),
            "approved replacement"
        );
        assert_eq!(
            retired.try_get::<Vec<u8>, _>("rotated_to_pubkey").unwrap(),
            new_pubkey
        );
        let tombstone_reason: String = sqlx::query_scalar(
            "SELECT reason FROM identity_revoked_keys WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(&old_pubkey)
        .fetch_one(&pool)
        .await
        .expect("key tombstone provenance");
        assert_eq!(tombstone_reason, "lost device");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn principal_can_be_disabled_before_first_enrollment() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        assert!(revoke_identity_principal(
            &pool,
            community,
            TEST_ISSUER,
            "never-enrolled",
            None,
            "employment ended",
        )
        .await
        .expect("persist principal tombstone"));
        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "never-enrolled",
                &random_pubkey(),
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("disabled principal result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn revoked_key_cannot_rebind_to_another_principal() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "first-principal",
            &pubkey,
            None,
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");
        revoke_identity_key(&pool, community, &pubkey, None, "compromised key")
            .await
            .expect("revoke key");

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                TEST_ISSUER,
                "different-principal",
                &pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("revoked key result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn legacy_revoked_key_history_blocks_cross_principal_rebind() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let pubkey = random_pubkey();
        bind_or_validate_identity(
            &pool,
            community,
            TEST_ISSUER,
            "legacy-principal",
            &pubkey,
            None,
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create binding");
        sqlx::query(
            "UPDATE identity_bindings SET revoked_at = NOW(), revoked_reason = 'legacy revoke' \
             WHERE community_id = $1 AND issuer = $2 AND uid = $3 AND pubkey = $4",
        )
        .bind(community.as_uuid())
        .bind(TEST_ISSUER)
        .bind("legacy-principal")
        .bind(&pubkey)
        .execute(&pool)
        .await
        .expect("simulate pre-lifecycle revocation");

        assert_eq!(
            bind_or_validate_identity(
                &pool,
                community,
                "https://other-idp.example",
                "different-principal",
                &pubkey,
                None,
                SOURCE_DB_BINDING,
            )
            .await
            .expect("legacy key tombstone result"),
            BindIdentityResult::Revoked
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bind_identity_qualifies_same_uid_by_issuer() {
        let pool = setup_pool().await;
        let community = make_community(&pool).await;
        let first_pubkey = random_pubkey();
        let second_pubkey = random_pubkey();

        let first = bind_or_validate_identity(
            &pool,
            community,
            "https://issuer-a.example",
            "shared-subject",
            &first_pubkey,
            Some("first@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create first issuer binding");
        let second = bind_or_validate_identity(
            &pool,
            community,
            "https://issuer-b.example",
            "shared-subject",
            &second_pubkey,
            Some("second@example.com"),
            SOURCE_DB_BINDING,
        )
        .await
        .expect("create second issuer binding");

        assert_eq!(first, BindIdentityResult::Created);
        assert_eq!(second, BindIdentityResult::Created);
        assert_eq!(
            get_active_identity_binding_by_pubkey(&pool, community, &second_pubkey)
                .await
                .expect("lookup second binding")
                .expect("second binding exists")
                .issuer,
            "https://issuer-b.example"
        );
    }
}
