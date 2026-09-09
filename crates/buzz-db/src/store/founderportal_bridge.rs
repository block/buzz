//! FounderPortal-owned collaboration bridge mapping persistence.
//!
//! This module owns cross-system identifiers and lifecycle state only. It does
//! not decide FounderPortal tenant membership or Buzz collaboration access.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::{error::DbError, Result};

const COMMUNITY_STATUSES: &[&str] = &[
    "provisioning", "active", "revoking", "revoked", "error", "archived",
];
const IDENTITY_STATUSES: &[&str] =
    &["provisioning", "active", "revoking", "revoked", "error"];

/// A tenant-to-community bridge mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunityMapping {
    /// FounderPortal tenant identifier.
    pub tenant_id: String,
    /// Buzz community identifier, absent while initial provisioning has not allocated one.
    pub buzz_community_id: Option<Uuid>,
    /// Mapping lifecycle state.
    pub status: String,
    /// Bounded non-sensitive error classification.
    pub last_error_code: Option<String>,
    /// Optimistic concurrency revision.
    pub revision: i64,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

/// A tenant-scoped FounderPortal actor to Buzz public-key mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IdentityMapping {
    /// FounderPortal tenant identifier.
    pub tenant_id: String,
    /// Actor class (`human` or `agent`).
    pub actor_type: String,
    /// FounderPortal-owned actor identifier.
    pub actor_id: String,
    /// Lowercase 64-character hexadecimal Buzz public key. This is not secret material.
    pub buzz_pubkey: String,
    /// Mapping lifecycle state.
    pub status: String,
    /// Optimistic concurrency revision.
    pub revision: i64,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last mutation timestamp.
    pub updated_at: DateTime<Utc>,
}

/// A fail-closed bridge mapping conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingConflict {
    /// The logical key already maps to different immutable coordinates.
    ImmutableCoordinates,
    /// The supplied lifecycle state or identifier is invalid.
    InvalidInput,
    /// A compare-and-swap update used a stale revision.
    StaleRevision,
    /// No mapping exists at the requested logical coordinate.
    NotFound,
}

/// Result of ensuring one mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureMapping<T> {
    /// This call inserted the mapping.
    Created(T),
    /// The exact logical mapping already existed.
    Existing(T),
    /// A database-enforced or verified conflict was found.
    Conflict(MappingConflict),
}

/// One reconciliation finding. Diagnostics report but never repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MappingDiagnostic {
    /// Stable diagnostic class.
    pub code: String,
    /// Affected table.
    pub table_name: String,
    /// Non-sensitive logical key details.
    pub details: serde_json::Value,
}

fn clean(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty() && trimmed == value).then_some(trimmed)
}

fn valid_pubkey(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_error_code(value: Option<&str>) -> bool {
    match value {
        None => true,
        Some(code) => clean(code).is_some() && code.chars().count() <= 128,
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.is_unique_violation())
}

fn community_row(row: sqlx::postgres::PgRow) -> Result<CommunityMapping> {
    Ok(CommunityMapping {
        tenant_id: row.try_get("tenant_id")?,
        buzz_community_id: row.try_get("buzz_community_id")?,
        status: row.try_get("status")?,
        last_error_code: row.try_get("last_error_code")?,
        revision: row.try_get("revision")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn identity_row(row: sqlx::postgres::PgRow) -> Result<IdentityMapping> {
    Ok(IdentityMapping {
        tenant_id: row.try_get("tenant_id")?,
        actor_type: row.try_get("actor_type")?,
        actor_id: row.try_get("actor_id")?,
        buzz_pubkey: row.try_get("buzz_pubkey")?,
        status: row.try_get("status")?,
        revision: row.try_get("revision")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Read one community mapping by FounderPortal tenant.
pub async fn get_community_mapping(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Option<CommunityMapping>> {
    let Some(tenant_id) = clean(tenant_id) else {
        return Err(DbError::InvalidData("tenant_id is required".into()));
    };
    sqlx::query(
        "SELECT tenant_id, buzz_community_id, status, last_error_code, revision, created_at, updated_at FROM founderportal_bridge.community_mappings WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .map(community_row)
    .transpose()
}

/// Ensure a tenant-to-community mapping without overwriting conflicting coordinates.
pub async fn ensure_community_mapping(
    pool: &PgPool,
    tenant_id: &str,
    buzz_community_id: Option<Uuid>,
    status: &str,
    last_error_code: Option<&str>,
) -> Result<EnsureMapping<CommunityMapping>> {
    let Some(tenant_id) = clean(tenant_id) else {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    };
    if !COMMUNITY_STATUSES.contains(&status)
        || (status == "provisioning" && buzz_community_id.is_some())
        || (["active", "revoking", "revoked", "archived"].contains(&status)
            && buzz_community_id.is_none())
        || !valid_error_code(last_error_code)
    {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    }

    let inserted = sqlx::query(
        r#"
        INSERT INTO founderportal_bridge.community_mappings
            (tenant_id, buzz_community_id, status, last_error_code)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT DO NOTHING
        RETURNING tenant_id, buzz_community_id, status, last_error_code,
                  revision, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(buzz_community_id)
    .bind(status)
    .bind(last_error_code)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = inserted {
        return Ok(EnsureMapping::Created(community_row(row)?));
    }

    let existing = get_community_mapping(pool, tenant_id).await?;
    match existing {
        Some(row)
            if row.buzz_community_id == buzz_community_id
                && row.status == status
                && row.last_error_code.as_deref() == last_error_code =>
        {
            Ok(EnsureMapping::Existing(row))
        }
        _ => Ok(EnsureMapping::Conflict(
            MappingConflict::ImmutableCoordinates,
        )),
    }
}

/// Move a community mapping state using an exact revision compare-and-swap.
pub async fn update_community_mapping(
    pool: &PgPool,
    tenant_id: &str,
    expected_revision: i64,
    status: &str,
    buzz_community_id: Option<Uuid>,
    last_error_code: Option<&str>,
) -> Result<EnsureMapping<CommunityMapping>> {
    let Some(tenant_id) = clean(tenant_id) else {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    };
    if expected_revision < 1
        || !COMMUNITY_STATUSES.contains(&status)
        || (status == "provisioning" && buzz_community_id.is_some())
        || (["active", "revoking", "revoked", "archived"].contains(&status)
            && buzz_community_id.is_none())
        || !valid_error_code(last_error_code)
    {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    }
    let updated = sqlx::query(
        r#"
        UPDATE founderportal_bridge.community_mappings
        SET buzz_community_id = $3, status = $4, last_error_code = $5,
            revision = revision + 1, updated_at = now()
        WHERE tenant_id = $1 AND revision = $2
        RETURNING tenant_id, buzz_community_id, status, last_error_code,
                  revision, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(expected_revision)
    .bind(buzz_community_id)
    .bind(status)
    .bind(last_error_code)
    .fetch_optional(pool)
    .await;
    match updated {
        Ok(Some(row)) => Ok(EnsureMapping::Existing(community_row(row)?)),
        Err(error) if is_unique_violation(&error) => Ok(EnsureMapping::Conflict(
            MappingConflict::ImmutableCoordinates,
        )),
        Err(error) => Err(error.into()),
        Ok(None) => match get_community_mapping(pool, tenant_id).await? {
            Some(_) => Ok(EnsureMapping::Conflict(MappingConflict::StaleRevision)),
            None => Ok(EnsureMapping::Conflict(MappingConflict::NotFound)),
        },
    }
}

/// Read one identity mapping by its FounderPortal actor coordinate.
pub async fn get_identity_mapping(
    pool: &PgPool,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
) -> Result<Option<IdentityMapping>> {
    if clean(tenant_id).is_none()
        || !["human", "agent"].contains(&actor_type)
        || clean(actor_id).is_none()
    {
        return Err(DbError::InvalidData(
            "valid tenant_id, actor_type, and actor_id are required".into(),
        ));
    }
    sqlx::query(
        "SELECT tenant_id, actor_type, actor_id, buzz_pubkey, status, revision, created_at, updated_at FROM founderportal_bridge.identity_mappings WHERE tenant_id = $1 AND actor_type = $2 AND actor_id = $3",
    )
    .bind(tenant_id)
    .bind(actor_type)
    .bind(actor_id)
    .fetch_optional(pool)
    .await?
    .map(identity_row)
    .transpose()
}

/// Ensure an actor-to-public-key mapping without overwriting conflicts.
pub async fn ensure_identity_mapping(
    pool: &PgPool,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
    buzz_pubkey: &str,
    status: &str,
) -> Result<EnsureMapping<IdentityMapping>> {
    if clean(tenant_id).is_none()
        || !["human", "agent"].contains(&actor_type)
        || clean(actor_id).is_none()
        || !valid_pubkey(buzz_pubkey)
        || !IDENTITY_STATUSES.contains(&status)
    {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    }
    let inserted = sqlx::query(
        r#"
        INSERT INTO founderportal_bridge.identity_mappings
            (tenant_id, actor_type, actor_id, buzz_pubkey, status)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT DO NOTHING
        RETURNING tenant_id, actor_type, actor_id, buzz_pubkey, status,
                  revision, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(actor_type)
    .bind(actor_id)
    .bind(buzz_pubkey)
    .bind(status)
    .fetch_optional(pool)
    .await?;
    if let Some(row) = inserted {
        return Ok(EnsureMapping::Created(identity_row(row)?));
    }
    let existing = get_identity_mapping(pool, tenant_id, actor_type, actor_id).await?;
    match existing {
        Some(row) if row.buzz_pubkey == buzz_pubkey && row.status == status => {
            Ok(EnsureMapping::Existing(row))
        }
        _ => Ok(EnsureMapping::Conflict(
            MappingConflict::ImmutableCoordinates,
        )),
    }
}

/// Move an identity mapping state using an exact revision compare-and-swap.
pub async fn update_identity_mapping_status(
    pool: &PgPool,
    tenant_id: &str,
    actor_type: &str,
    actor_id: &str,
    expected_revision: i64,
    status: &str,
) -> Result<EnsureMapping<IdentityMapping>> {
    let Some(tenant_id) = clean(tenant_id) else {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    };
    if !["human", "agent"].contains(&actor_type)
        || clean(actor_id).is_none()
        || expected_revision < 1
        || !IDENTITY_STATUSES.contains(&status)
    {
        return Ok(EnsureMapping::Conflict(MappingConflict::InvalidInput));
    }
    let updated = sqlx::query(
        r#"
        UPDATE founderportal_bridge.identity_mappings
        SET status = $5, revision = revision + 1, updated_at = now()
        WHERE tenant_id = $1 AND actor_type = $2 AND actor_id = $3
          AND revision = $4
        RETURNING tenant_id, actor_type, actor_id, buzz_pubkey, status,
                  revision, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(actor_type)
    .bind(actor_id)
    .bind(expected_revision)
    .bind(status)
    .fetch_optional(pool)
    .await?;
    match updated {
        Some(row) => Ok(EnsureMapping::Existing(identity_row(row)?)),
        None => match get_identity_mapping(pool, tenant_id, actor_type, actor_id).await? {
            Some(_) => Ok(EnsureMapping::Conflict(MappingConflict::StaleRevision)),
            None => Ok(EnsureMapping::Conflict(MappingConflict::NotFound)),
        },
    }
}

/// Report malformed or duplicate bridge state without changing it.
///
/// PostgreSQL constraints make these findings empty during normal operation;
/// the explicit checks support integrity diagnostics if constraints are ever
/// disabled or the schema is restored incorrectly.
pub async fn diagnose_mappings(pool: &PgPool) -> Result<Vec<MappingDiagnostic>> {
    let rows = sqlx::query(
        r#"
        SELECT code, table_name, details
        FROM (
            SELECT 'duplicate_tenant_mapping' AS code, 'community_mappings' AS table_name,
                   jsonb_build_object('tenant_id', tenant_id, 'count', count(*)) AS details
            FROM founderportal_bridge.community_mappings GROUP BY tenant_id HAVING count(*) > 1
            UNION ALL
            SELECT 'duplicate_community_mapping', 'community_mappings',
                   jsonb_build_object('buzz_community_id', buzz_community_id, 'count', count(*))
            FROM founderportal_bridge.community_mappings
            WHERE buzz_community_id IS NOT NULL GROUP BY buzz_community_id HAVING count(*) > 1
            UNION ALL
            SELECT 'duplicate_actor_mapping', 'identity_mappings',
                   jsonb_build_object('tenant_id', tenant_id, 'actor_type', actor_type,
                                      'actor_id', actor_id, 'count', count(*))
            FROM founderportal_bridge.identity_mappings
            GROUP BY tenant_id, actor_type, actor_id HAVING count(*) > 1
            UNION ALL
            SELECT 'duplicate_pubkey_mapping', 'identity_mappings',
                   jsonb_build_object('tenant_id', tenant_id, 'buzz_pubkey', buzz_pubkey,
                                      'count', count(*))
            FROM founderportal_bridge.identity_mappings
            GROUP BY tenant_id, buzz_pubkey HAVING count(*) > 1
            UNION ALL
            SELECT 'malformed_community_mapping', 'community_mappings',
                   jsonb_build_object('tenant_id', tenant_id)
            FROM founderportal_bridge.community_mappings
            WHERE tenant_id IS NULL OR btrim(tenant_id) = '' OR tenant_id <> btrim(tenant_id)
               OR status NOT IN ('provisioning','active','revoking','revoked','error','archived')
               OR revision IS NULL OR revision <= 0 OR created_at IS NULL OR updated_at IS NULL
               OR (last_error_code IS NOT NULL AND (
                    last_error_code = '' OR last_error_code <> btrim(last_error_code)
                    OR length(last_error_code) > 128))
               OR (status = 'provisioning' AND buzz_community_id IS NOT NULL)
               OR (status IN ('active','revoking','revoked','archived') AND buzz_community_id IS NULL)
            UNION ALL
            SELECT 'malformed_identity_mapping', 'identity_mappings',
                   jsonb_build_object('tenant_id', tenant_id, 'actor_type', actor_type,
                                      'actor_id', actor_id)
            FROM founderportal_bridge.identity_mappings
            WHERE tenant_id IS NULL OR btrim(tenant_id) = '' OR tenant_id <> btrim(tenant_id)
               OR actor_type NOT IN ('human','agent')
               OR actor_id IS NULL OR btrim(actor_id) = '' OR actor_id <> btrim(actor_id)
               OR buzz_pubkey IS NULL OR buzz_pubkey !~ '^[0-9a-f]{64}$'
               OR status NOT IN ('provisioning','active','revoking','revoked','error')
               OR revision IS NULL OR revision <= 0 OR created_at IS NULL OR updated_at IS NULL
        ) findings
        ORDER BY code, details::text
        "#,
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(MappingDiagnostic {
                code: row.try_get("code")?,
                table_name: row.try_get("table_name")?,
                details: row.try_get("details")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    async fn pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect isolated PostgreSQL test database")
    }

    fn key(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn community_mapping_constraints_and_concurrency_are_atomic() {
        let pool = pool().await;
        let community = Uuid::new_v4();
        let (a, b) = tokio::join!(
            ensure_community_mapping(&pool, "tenant-a", Some(community), "active", None),
            ensure_community_mapping(&pool, "tenant-a", Some(community), "active", None),
        );
        let results = [a.expect("first create"), b.expect("second create")];
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Created(_))).count(), 1);
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Existing(_))).count(), 1);

        let second_community = Uuid::new_v4();
        let (tenant_b, tenant_c) = tokio::join!(
            ensure_community_mapping(&pool, "tenant-b", Some(second_community), "active", None),
            ensure_community_mapping(&pool, "tenant-c", Some(second_community), "active", None),
        );
        let competing = [tenant_b.expect("tenant b"), tenant_c.expect("tenant c")];
        assert_eq!(competing.iter().filter(|r| matches!(r, EnsureMapping::Created(_))).count(), 1);
        assert_eq!(competing.iter().filter(|r| matches!(r, EnsureMapping::Conflict(MappingConflict::ImmutableCoordinates))).count(), 1);

        let row = get_community_mapping(&pool, "tenant-a").await.expect("read").expect("exists");
        assert_eq!(row.buzz_community_id, Some(community));
        let (winner, loser) = tokio::join!(
            update_community_mapping(&pool, "tenant-a", row.revision, "revoking", Some(community), None),
            update_community_mapping(&pool, "tenant-a", row.revision, "revoked", Some(community), None),
        );
        let updates = [winner.expect("winner"), loser.expect("loser")];
        assert_eq!(updates.iter().filter(|r| matches!(r, EnsureMapping::Existing(_))).count(), 1);
        assert_eq!(updates.iter().filter(|r| matches!(r, EnsureMapping::Conflict(MappingConflict::StaleRevision))).count(), 1);
        let current = get_community_mapping(&pool, "tenant-a").await.expect("read").expect("exists");
        update_community_mapping(&pool, "tenant-a", current.revision, "archived", Some(community), None)
            .await.expect("archive");
        assert!(matches!(
            ensure_community_mapping(&pool, "tenant-z", Some(community), "active", None).await.expect("conflict"),
            EnsureMapping::Conflict(MappingConflict::ImmutableCoordinates)
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn identity_mapping_constraints_and_concurrency_are_atomic() {
        let pool = pool().await;
        let pubkey = key(1);
        let (a, b) = tokio::join!(
            ensure_identity_mapping(&pool, "tenant-a", "human", "user-1", &pubkey, "active"),
            ensure_identity_mapping(&pool, "tenant-a", "human", "user-1", &pubkey, "active"),
        );
        let results = [a.expect("first create"), b.expect("second create")];
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Created(_))).count(), 1);
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Existing(_))).count(), 1);

        let actor_race = tokio::join!(
            ensure_identity_mapping(&pool, "tenant-a", "agent", "agent-1", &key(2), "active"),
            ensure_identity_mapping(&pool, "tenant-a", "agent", "agent-1", &key(3), "active"),
        );
        assert_eq!(
            [actor_race.0.expect("a"), actor_race.1.expect("b")]
                .iter().filter(|r| matches!(r, EnsureMapping::Created(_))).count(),
            1
        );

        let shared = key(4);
        let pubkey_race = tokio::join!(
            ensure_identity_mapping(&pool, "tenant-a", "human", "user-2", &shared, "active"),
            ensure_identity_mapping(&pool, "tenant-a", "human", "user-3", &shared, "active"),
        );
        assert_eq!(
            [pubkey_race.0.expect("a"), pubkey_race.1.expect("b")]
                .iter().filter(|r| matches!(r, EnsureMapping::Created(_))).count(),
            1
        );

        assert!(matches!(
            ensure_identity_mapping(&pool, "tenant-a", "human", "user-1", &key(5), "active").await.expect("conflict"),
            EnsureMapping::Conflict(MappingConflict::ImmutableCoordinates)
        ));
        assert!(matches!(
            ensure_identity_mapping(&pool, "tenant-b", "human", "user-1", &key(6), "active").await.expect("tenant b"),
            EnsureMapping::Created(_)
        ));
        assert!(matches!(
            ensure_identity_mapping(&pool, "tenant-b", "human", "user-2", &shared, "active")
                .await
                .expect("same pubkey in another tenant"),
            EnsureMapping::Created(_)
        ));
        assert!(get_identity_mapping(&pool, " tenant-b", "human", "user-1")
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lifecycle_validation_stale_updates_and_diagnostics_fail_closed() {
        let pool = pool().await;
        assert_eq!(
            ensure_community_mapping(&pool, "", None, "provisioning", None).await.expect("input"),
            EnsureMapping::Conflict(MappingConflict::InvalidInput)
        );
        assert_eq!(
            ensure_community_mapping(&pool, "tenant-a", None, "active", None).await.expect("input"),
            EnsureMapping::Conflict(MappingConflict::InvalidInput)
        );
        assert_eq!(
            ensure_identity_mapping(&pool, "tenant-a", "employee", "user", &key(7), "active").await.expect("input"),
            EnsureMapping::Conflict(MappingConflict::InvalidInput)
        );
        assert_eq!(
            ensure_identity_mapping(&pool, "tenant-a", "human", "user", "nsec-secret", "active").await.expect("input"),
            EnsureMapping::Conflict(MappingConflict::InvalidInput)
        );

        let created = ensure_identity_mapping(&pool, "tenant-a", "human", "user", &key(7), "active")
            .await.expect("create");
        let revision = match created { EnsureMapping::Created(row) => row.revision, _ => panic!("created") };
        let (first, stale) = tokio::join!(
            update_identity_mapping_status(&pool, "tenant-a", "human", "user", revision, "revoking"),
            update_identity_mapping_status(&pool, "tenant-a", "human", "user", revision, "revoked"),
        );
        let results = [first.expect("first"), stale.expect("second")];
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Existing(_))).count(), 1);
        assert_eq!(results.iter().filter(|r| matches!(r, EnsureMapping::Conflict(MappingConflict::StaleRevision))).count(), 1);
        assert!(diagnose_mappings(&pool).await.expect("diagnostics").is_empty());
    }
}
