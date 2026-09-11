//! Community lifecycle and host-map persistence.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::{relay_members, Db, DbError, Result};

/// Community host-map row returned by [`Db::lookup_community_by_host`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Normalized host that maps to this community.
    pub host: String,
}

/// Community row returned by idempotent community ensure/create operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredCommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Normalized host that maps to this community.
    pub host: String,
    /// True only when this call inserted the `communities` row.
    pub created: bool,
}

/// Community row returned by an atomic create-with-owner operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedCommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Normalized host stored for the community.
    pub host: String,
}

/// Result of atomically creating a community with its initial owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateCommunityWithOwnerResult {
    /// The community was created, or an identical retried create found it.
    Created(CreatedCommunityRecord),
    /// The host already belongs to another owner.
    HostExists,
    /// The intended owner already owns the maximum number of communities.
    LimitReached,
}

/// Community row returned by operator-plane ownership reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedCommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Normalized host that maps to this community.
    pub host: String,
    /// When the community row was created.
    pub created_at: DateTime<Utc>,
    /// When the community was archived; absent while active.
    pub archived_at: Option<DateTime<Utc>>,
}

/// Community row returned by an owner-authorized archive operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedCommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Reserved canonical host.
    pub host: String,
    /// Durable first-archive timestamp.
    pub archived_at: DateTime<Utc>,
}

/// Community row returned by an owner-authorized unarchive operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnarchivedCommunityRecord {
    /// Stable server-resolved community id.
    pub id: CommunityId,
    /// Reserved canonical host restored to active admission.
    pub host: String,
}

impl Db {
    /// Returns the community mapped to a normalized request host, if one exists.
    ///
    /// The caller owns host normalization and turns `None` into the fail-closed
    /// request/connection error. buzz-db only reads the durable host map.
    #[datastore_span(name = "lookup_community_by_host", system = "postgresql")]
    pub async fn lookup_community_by_host(
        &self,
        normalized_host: &str,
    ) -> Result<Option<CommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::TenantResolution,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT id, host
            FROM communities
            WHERE lower(host) = lower($1)
              AND archived_at IS NULL
              AND deleted_at IS NULL
              AND deletion_state = 'active'
            "#,
        )
        .bind(normalized_host)
        .fetch_optional(&mut *connection)
        .await?;

        row.map(|row| {
            let id: Uuid = row.try_get("id")?;
            let host: String = row.try_get("host")?;

            Ok(CommunityRecord {
                id: CommunityId::from_uuid(id),
                host,
            })
        })
        .transpose()
    }

    /// Returns whether a community id still exists in the active lifecycle state.
    #[datastore_span(name = "is_community_active", system = "postgresql")]
    pub async fn is_community_active(&self, community_id: CommunityId) -> Result<bool> {
        self.is_community_active_with_operation(
            community_id,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Background lifecycle revalidation variant of [`Self::is_community_active`].
    #[datastore_span(name = "is_community_active_for_maintenance", system = "postgresql")]
    pub async fn is_community_active_for_maintenance(
        &self,
        community_id: CommunityId,
    ) -> Result<bool> {
        self.is_community_active_with_operation(
            community_id,
            crate::observability::WriterOperation::Maintenance,
        )
        .await
    }

    async fn is_community_active_with_operation(
        &self,
        community_id: CommunityId,
        operation: crate::observability::WriterOperation,
    ) -> Result<bool> {
        let mut connection = crate::observability::acquire_writer(&self.pool, operation).await?;
        let active = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM communities WHERE id = $1 AND archived_at IS NULL AND deleted_at IS NULL AND deletion_state = 'active')",
        )
        .bind(community_id.as_uuid())
        .fetch_one(&mut *connection)
        .await?;
        Ok(active)
    }

    /// Returns a community by host regardless of lifecycle state. Operator-plane only.
    #[datastore_span(
        name = "lookup_community_by_host_for_management",
        system = "postgresql"
    )]
    pub async fn lookup_community_by_host_for_management(
        &self,
        normalized_host: &str,
    ) -> Result<Option<CommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query("SELECT id, host FROM communities WHERE lower(host) = lower($1)")
            .bind(normalized_host)
            .fetch_optional(&mut *connection)
            .await?;
        row.map(|row| {
            Ok(CommunityRecord {
                id: CommunityId::from_uuid(row.try_get("id")?),
                host: row.try_get("host")?,
            })
        })
        .transpose()
    }

    /// Lists communities where `owner_pubkey` currently holds the `owner` role.
    ///
    /// This is an operator-plane helper, not a tenant-scoped data-plane read:
    /// callers must gate it on deployment-level operator auth before exposing it.
    #[datastore_span(name = "list_communities_owned_by", system = "postgresql")]
    pub async fn list_communities_owned_by(
        &self,
        owner_pubkey: &str,
    ) -> Result<Vec<OwnedCommunityRecord>> {
        let owner_pubkey = owner_pubkey.to_ascii_lowercase();
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT c.id, c.host, c.created_at, c.archived_at
            FROM communities c
            JOIN relay_members rm ON rm.community_id = c.id
            WHERE rm.pubkey = $1
              AND rm.role = 'owner'
            ORDER BY c.created_at ASC, c.host ASC
            "#,
        )
        .bind(owner_pubkey)
        .fetch_all(&mut *connection)
        .await?;

        rows.into_iter()
            .map(|row| {
                let id: Uuid = row.try_get("id")?;
                let host: String = row.try_get("host")?;
                let created_at: DateTime<Utc> = row.try_get("created_at")?;
                let archived_at: Option<DateTime<Utc>> = row.try_get("archived_at")?;
                Ok(OwnedCommunityRecord {
                    id: CommunityId::from_uuid(id),
                    host,
                    created_at,
                    archived_at,
                })
            })
            .collect()
    }

    /// Returns the normalized host mapped to a community id, if the community
    /// exists.
    ///
    /// The reverse of [`lookup_community_by_host`]: used by side-effect
    /// producers that already hold a server-resolved `CommunityId` (e.g. the
    /// workflow action sink running a run owned by some community) and need a
    /// fully-formed [`buzz_core::tenant::TenantContext`] — host included — to
    /// fan out under *that* community rather than the deployment default. The
    /// community is authoritative; the host is read back for labelling only and
    /// is never used to re-derive the community.
    #[datastore_span(name = "lookup_community_host", system = "postgresql")]
    pub async fn lookup_community_host(&self, community_id: CommunityId) -> Result<Option<String>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::TenantResolution,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT host
            FROM communities
            WHERE id = $1
              AND archived_at IS NULL
              AND deleted_at IS NULL
              AND deletion_state = 'active'
            "#,
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&mut *connection)
        .await?;

        row.map(|row| {
            let host: String = row.try_get("host")?;
            Ok(host)
        })
        .transpose()
    }

    /// Returns the community's workspace icon (NIP-11 `icon`), if set.
    ///
    /// Set by relay admins/owners via the kind:9033 command; the value is
    /// validated and size-capped at that write path.
    #[datastore_span(name = "get_community_icon", system = "postgresql")]
    pub async fn get_community_icon(&self, community_id: CommunityId) -> Result<Option<String>> {
        let row = sqlx::query(
            r#"
            SELECT icon
            FROM communities
            WHERE id = $1
            "#,
        )
        .bind(community_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row
            .map(|row| row.try_get::<Option<String>, _>("icon"))
            .transpose()?
            .flatten()
            .filter(|icon| !icon.is_empty()))
    }

    /// Sets or clears (`None`) the community's workspace icon.
    #[datastore_span(name = "set_community_icon", system = "postgresql")]
    pub async fn set_community_icon(
        &self,
        community_id: CommunityId,
        icon: Option<&str>,
    ) -> Result<()> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::query(
            r#"
            UPDATE communities
            SET icon = $2
            WHERE id = $1
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(icon)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    /// Ensure a configured community host exists and return its row.
    ///
    /// This is the startup/config seeding path for N=1 deployments. Migrations
    /// create the schema only; deployment-specific hosts are not hardcoded into
    /// schema history.
    #[datastore_span(name = "ensure_configured_community", system = "postgresql")]
    pub async fn ensure_configured_community(
        &self,
        normalized_host: &str,
    ) -> Result<EnsuredCommunityRecord> {
        self.ensure_configured_community_with_operation(
            normalized_host,
            crate::observability::WriterOperation::Authorization,
        )
        .await
    }

    /// Ensure the deployment-configured community during process bootstrap.
    #[datastore_span(
        name = "ensure_configured_community_for_bootstrap",
        system = "postgresql"
    )]
    pub async fn ensure_configured_community_for_bootstrap(
        &self,
        normalized_host: &str,
    ) -> Result<EnsuredCommunityRecord> {
        self.ensure_configured_community_with_operation(
            normalized_host,
            crate::observability::WriterOperation::Bootstrap,
        )
        .await
    }

    async fn ensure_configured_community_with_operation(
        &self,
        normalized_host: &str,
        operation: crate::observability::WriterOperation,
    ) -> Result<EnsuredCommunityRecord> {
        let mut connection = crate::observability::acquire_writer(&self.pool, operation).await?;
        let row = sqlx::query(
            r#"
            INSERT INTO communities (host)
            VALUES ($1)
            ON CONFLICT (lower(host)) DO UPDATE SET host = communities.host
            WHERE communities.deletion_state = 'active'
              AND communities.deleted_at IS NULL
            RETURNING id, host, (xmax = 0) AS created
            "#,
        )
        .bind(normalized_host)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| {
            DbError::AccessDenied(format!(
                "community host {normalized_host:?} is permanently tombstoned"
            ))
        })?;

        let id: Uuid = row.try_get("id")?;
        let host: String = row.try_get("host")?;
        let created: bool = row.try_get("created")?;

        Ok(EnsuredCommunityRecord {
            id: CommunityId::from_uuid(id),
            host,
            created,
        })
    }

    /// Atomically creates a community and its initial owner.
    ///
    /// Holds a per-owner advisory lock while enforcing the ownership limit.
    /// Identical create retries return the original record; host collisions and
    /// limit failures remain distinguishable to the operator API.
    #[datastore_span(name = "create_community_with_owner", system = "postgresql")]
    pub async fn create_community_with_owner(
        &self,
        normalized_host: &str,
        owner_pubkey: &str,
    ) -> Result<CreateCommunityWithOwnerResult> {
        let owner_pubkey = owner_pubkey.to_ascii_lowercase();
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;

        // Serialize on the owner pubkey so concurrent creates to the same
        // owner cannot both pass the ownership count check.
        crate::observability::observe_advisory_lock(
            crate::observability::LockType::Membership,
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(relay_members::owner_count_advisory_lock_key(&owner_pubkey))
                .execute(&mut *tx),
        )
        .await?;

        let row = sqlx::query(
            r#"
            INSERT INTO communities (host)
            VALUES ($1)
            ON CONFLICT (lower(host)) DO NOTHING
            RETURNING id, host
            "#,
        )
        .bind(normalized_host)
        .fetch_optional(&mut *tx)
        .await?;

        let (id, host) = if let Some(row) = row {
            let id: Uuid = row.try_get("id")?;
            let host: String = row.try_get("host")?;

            // Enforce the limit before inserting the new owner row.
            let owned_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM relay_members WHERE pubkey = $1 AND role = 'owner'",
            )
            .bind(&owner_pubkey)
            .fetch_one(&mut *tx)
            .await?;

            if owned_count >= relay_members::max_communities_per_owner() {
                tx.rollback().await?;
                return Ok(CreateCommunityWithOwnerResult::LimitReached);
            }

            sqlx::query(
                "INSERT INTO relay_members (community_id, pubkey, role, added_by) VALUES ($1, $2, 'owner', NULL)",
            )
            .bind(id)
            .bind(&owner_pubkey)
            .execute(&mut *tx)
            .await?;
            (id, host)
        } else {
            let existing = sqlx::query(
                r#"
                SELECT c.id, c.host
                FROM communities c
                JOIN relay_members rm ON rm.community_id = c.id
                WHERE lower(c.host) = lower($1)
                  AND lower(rm.pubkey) = lower($2)
                  AND rm.role = 'owner'
                  AND c.archived_at IS NULL
                  AND c.deletion_state = 'active'
                  AND c.deleted_at IS NULL
                "#,
            )
            .bind(normalized_host)
            .bind(&owner_pubkey)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(existing) = existing else {
                tx.rollback().await?;
                return Ok(CreateCommunityWithOwnerResult::HostExists);
            };
            (existing.try_get("id")?, existing.try_get("host")?)
        };

        tx.commit().await?;
        Ok(CreateCommunityWithOwnerResult::Created(
            CreatedCommunityRecord {
                id: CommunityId::from_uuid(id),
                host,
            },
        ))
    }

    /// Idempotently archives a community when the asserted pubkey is its current owner.
    #[datastore_span(name = "archive_community_owned_by", system = "postgresql")]
    pub async fn archive_community_owned_by(
        &self,
        normalized_host: &str,
        owner_pubkey: &str,
        protected_deployment_host: &str,
    ) -> Result<Option<ArchivedCommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query(
            r#"UPDATE communities c
               SET archived_at = COALESCE(c.archived_at, now())
               FROM relay_members rm
               WHERE lower(c.host) = lower($1)
                 AND rm.community_id = c.id
                 AND lower(rm.pubkey) = lower($2)
                 AND rm.role = 'owner'
                 AND lower(c.host) <> lower($3)
                 AND c.deletion_state = 'active'
                 AND c.deleted_at IS NULL
               RETURNING c.id, c.host, c.archived_at"#,
        )
        .bind(normalized_host)
        .bind(owner_pubkey)
        .bind(protected_deployment_host)
        .fetch_optional(&mut *connection)
        .await?;
        row.map(|row| {
            Ok(ArchivedCommunityRecord {
                id: CommunityId::from_uuid(row.try_get("id")?),
                host: row.try_get("host")?,
                archived_at: row.try_get("archived_at")?,
            })
        })
        .transpose()
    }

    /// Idempotently restores a community when the asserted pubkey is its current owner.
    #[datastore_span(name = "unarchive_community_owned_by", system = "postgresql")]
    pub async fn unarchive_community_owned_by(
        &self,
        normalized_host: &str,
        owner_pubkey: &str,
    ) -> Result<Option<UnarchivedCommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query(
            r#"UPDATE communities c
               SET archived_at = NULL
               FROM relay_members rm
               WHERE lower(c.host) = lower($1)
                 AND rm.community_id = c.id
                 AND lower(rm.pubkey) = lower($2)
                 AND rm.role = 'owner'
                 AND c.deletion_state = 'active'
                 AND c.deleted_at IS NULL
               RETURNING c.id, c.host"#,
        )
        .bind(normalized_host)
        .bind(owner_pubkey)
        .fetch_optional(&mut *connection)
        .await?;
        row.map(|row| {
            Ok(UnarchivedCommunityRecord {
                id: CommunityId::from_uuid(row.try_get("id")?),
                host: row.try_get("host")?,
            })
        })
        .transpose()
    }

    /// Returns the community that owns a channel, if the channel exists.
    ///
    /// Internal relay producers use this to derive tenant context from the row
    /// they are acting on, rather than falling back to an implicit default.
    #[datastore_span(name = "community_of_channel", system = "postgresql")]
    pub async fn community_of_channel(&self, channel_id: Uuid) -> Result<Option<CommunityId>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::TenantResolution,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT community_id
            FROM channels
            WHERE id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(channel_id)
        .fetch_optional(&mut *connection)
        .await?;

        row.map(|row| {
            let id: Uuid = row.try_get("community_id")?;
            Ok(CommunityId::from_uuid(id))
        })
        .transpose()
    }

    /// Batched version of [`Self::community_of_channel`]: given a list of
    /// channel UUIDs, returns a map from channel id → owning community
    /// for every channel that exists (soft-deletes excluded).
    ///
    /// Used by the runtime conformance read-seam emitters in `buzz-relay`:
    /// after a `query_events`/`get_events_by_ids` returns N rows, the
    /// emitter collects distinct `channel_id`s, calls this once, then
    /// projects each row's true community label independently of the
    /// fetch query's WHERE clause. That independence is what makes the
    /// `Inv_NonInterference` / `Inv_ReadConfinement` gate non-vacuous —
    /// a mutation that dropped `community_id = $X` from the fetch query
    /// would still let this helper return the row's true label, and the
    /// checker would see the mismatch.
    ///
    /// Channels missing from the result map (deleted or never existed)
    /// are intentionally not present rather than mapped to a default —
    /// callers MUST treat "channel-id not in map" as a coverage breach,
    /// never as "use the resolved community".
    #[datastore_span(name = "communities_of_channels", system = "postgresql")]
    pub async fn communities_of_channels(
        &self,
        channel_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, CommunityId>> {
        if channel_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT id, community_id
            FROM channels
            WHERE id = ANY($1)
              AND deleted_at IS NULL
            "#,
        )
        .bind(channel_ids)
        .fetch_all(&mut *connection)
        .await?;

        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let ch: Uuid = row.try_get("id")?;
            let cm: Uuid = row.try_get("community_id")?;
            out.insert(ch, CommunityId::from_uuid(cm));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod postgres_tests {
    //! Pin the load-bearing contract for `Db::communities_of_channels`:
    //! a channel id that does NOT exist MUST be absent from the result
    //! map, never mapped to a default. The relay-side read-row emitter
    //! relies on this — a missing entry triggers `MissingLookup →
    //! ImplBug{row_community_lookup_missing} → CoverageBreach`. If this
    //! helper ever started returning a default/zero entry for unknown
    //! channels, that fail-closed chain would go blind.
    use super::*;
    use sqlx::PgPool;

    async fn setup_db() -> Db {
        let database_url = crate::test_support::database_url();
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

    async fn insert_channel(pool: &PgPool, community_id: Uuid, channel_id: Uuid) {
        let creator: Vec<u8> = vec![0u8; 32];
        sqlx::query(
            r#"
            INSERT INTO channels
                (id, community_id, name, channel_type, visibility, created_by)
            VALUES
                ($1, $2, $3, 'stream'::channel_type, 'open'::channel_visibility, $4)
            "#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(format!("ch-{}", channel_id.simple()))
        .bind(&creator)
        .execute(pool)
        .await
        .expect("insert channel");
    }

    #[test]
    fn community_implementation_tests_and_spans_have_single_owners() {
        let community_source = include_str!("community.rs");
        let lib_source = include_str!("../lib.rs");
        let operations = [
            "lookup_community_by_host",
            "is_community_active",
            "lookup_community_by_host_for_management",
            "list_communities_owned_by",
            "lookup_community_host",
            "get_community_icon",
            "set_community_icon",
            "ensure_configured_community",
            "create_community_with_owner",
            "archive_community_owned_by",
            "unarchive_community_owned_by",
            "community_of_channel",
            "communities_of_channels",
        ];
        for operation in operations {
            let method = format!("pub async fn {operation}(");
            assert_eq!(
                community_source.matches(&method).count(),
                1,
                "{operation} implementation must live exactly once in community.rs",
            );
            assert!(
                !lib_source.contains(&method),
                "{operation} implementation must not remain in lib.rs",
            );

            let span = format!("name = \"{operation}\"");
            assert_eq!(
                community_source.matches(&span).count(),
                1,
                "{operation} must have exactly one datastore span",
            );
            assert!(
                !lib_source.contains(&span),
                "{operation} datastore span must not remain in lib.rs",
            );
        }

        let records = [
            "CommunityRecord",
            "EnsuredCommunityRecord",
            "CreatedCommunityRecord",
            "OwnedCommunityRecord",
            "ArchivedCommunityRecord",
            "UnarchivedCommunityRecord",
        ];
        for record in records {
            let declaration = format!("pub struct {record}");
            assert_eq!(community_source.matches(&declaration).count(), 1);
            assert!(!lib_source.contains(&declaration));
        }
        let result_declaration = format!("pub {} {}", "enum", "CreateCommunityWithOwnerResult");
        assert_eq!(community_source.matches(&result_declaration).count(), 1);
        assert!(!lib_source.contains(&result_declaration));

        let moved_tests = [
            "lookup_community_by_host_matches_case_insensitive_host_index",
            "create_community_with_owner_is_atomic_and_create_only",
            "unarchive_community_owned_by_restores_admission_idempotently",
            "create_community_with_owner_enforces_per_owner_limit",
            "concurrent_same_owner_create_returns_the_winning_row_to_both_callers",
            "ensure_configured_community_reports_insert_winner",
            "list_communities_owned_by_returns_only_owner_rows",
            "communities_of_channels_present_for_existing_absent_for_missing",
        ];
        for test in moved_tests {
            let declaration = format!("async fn {test}");
            assert_eq!(community_source.matches(&declaration).count(), 1);
            assert!(!lib_source.contains(&declaration));
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lookup_community_by_host_matches_case_insensitive_host_index() {
        let db = setup_db().await;
        let id = Uuid::new_v4();
        let lower_host = format!("lookup-community-{}.example", id.simple());
        let stored_host = lower_host.to_uppercase();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&stored_host)
            .execute(&db.pool)
            .await
            .expect("insert mixed-case community host");

        let found = db
            .lookup_community_by_host(&lower_host)
            .await
            .expect("lookup lower-case host")
            .expect("community found by lower-case host");
        assert_eq!(found.id, CommunityId::from_uuid(id));
        assert_eq!(found.host, stored_host);

        let found = db
            .lookup_community_by_host(&stored_host)
            .await
            .expect("lookup stored-case host")
            .expect("community found by stored-case host");
        assert_eq!(found.id, CommunityId::from_uuid(id));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_community_with_owner_is_atomic_and_create_only() {
        let db = setup_db().await;
        let host = format!("create-only-{}.example", Uuid::new_v4().simple());
        let owner = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let created = db
            .create_community_with_owner(&host, owner)
            .await
            .expect("create community");
        let CreateCommunityWithOwnerResult::Created(created) = created else {
            panic!("expected new community");
        };
        assert_eq!(created.host, host);
        let owner_role: Option<String> = sqlx::query_scalar(
            "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(created.id.as_uuid())
        .bind(owner)
        .fetch_optional(&db.pool)
        .await
        .expect("owner role");
        assert_eq!(owner_role.as_deref(), Some("owner"));

        let retry = db
            .create_community_with_owner(&host.to_ascii_uppercase(), owner)
            .await
            .expect("same-owner retry");
        assert_eq!(
            retry,
            CreateCommunityWithOwnerResult::Created(created.clone()),
            "retry returns the original row"
        );

        let collision = db
            .create_community_with_owner(&host, other)
            .await
            .expect("collision result");
        assert_eq!(collision, CreateCommunityWithOwnerResult::HostExists);
        let roles: Vec<(String, String)> = sqlx::query_as(
            "SELECT pubkey, role FROM relay_members WHERE community_id = $1 ORDER BY pubkey",
        )
        .bind(created.id.as_uuid())
        .fetch_all(&db.pool)
        .await
        .expect("community roles");
        assert_eq!(roles, vec![(owner.to_string(), "owner".to_string())]);

        db.bootstrap_owner(created.id, other)
            .await
            .expect("rotate owner");
        let post_rotation_retry = db
            .create_community_with_owner(&host, owner)
            .await
            .expect("post-rotation retry");
        assert_eq!(
            post_rotation_retry,
            CreateCommunityWithOwnerResult::HostExists
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unarchive_community_owned_by_restores_admission_idempotently() {
        let db = setup_db().await;
        let host = format!("unarchive-{}.example", Uuid::new_v4().simple());
        let owner = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let outsider = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let created = db
            .create_community_with_owner(&host, &owner)
            .await
            .expect("create community");
        let CreateCommunityWithOwnerResult::Created(created) = created else {
            panic!("expected new community");
        };

        let archived = db
            .archive_community_owned_by(&host, &owner, "protected.example")
            .await
            .expect("archive community")
            .expect("owned community");
        assert_eq!(archived.id, created.id);
        assert!(
            db.lookup_community_by_host(&host)
                .await
                .expect("active lookup")
                .is_none(),
            "archived communities must fail admission"
        );
        assert!(db
            .unarchive_community_owned_by(&host, &outsider)
            .await
            .expect("wrong-owner unarchive")
            .is_none());
        assert!(db
            .unarchive_community_owned_by("missing.example", &owner)
            .await
            .expect("unknown-host unarchive")
            .is_none());

        let restored = db
            .unarchive_community_owned_by(&host.to_ascii_uppercase(), &owner)
            .await
            .expect("unarchive community")
            .expect("owned community");
        assert_eq!(restored.id, created.id);
        assert_eq!(restored.host, host);
        assert_eq!(
            db.lookup_community_by_host(&host)
                .await
                .expect("restored lookup")
                .expect("active community")
                .id,
            created.id
        );
        assert_eq!(
            db.get_relay_member(created.id, &owner)
                .await
                .expect("owner lookup")
                .expect("owner remains")
                .role,
            "owner"
        );

        let retry = db
            .unarchive_community_owned_by(&host, &owner)
            .await
            .expect("idempotent retry")
            .expect("owned community");
        assert_eq!(retry, restored);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_community_with_owner_enforces_per_owner_limit() {
        let db = setup_db().await;
        let owner = format!("{:064x}", Uuid::new_v4().as_u128());

        // Fill the configured default ownership limit.
        for i in 0..crate::relay_members::MAX_COMMUNITIES_PER_OWNER {
            let host = format!("limit-test-{}-{}.example", i, Uuid::new_v4().simple());
            assert!(matches!(
                db.create_community_with_owner(&host, &owner)
                    .await
                    .expect("create community"),
                CreateCommunityWithOwnerResult::Created(_)
            ));
        }

        let host = format!("limit-test-overflow-{}.example", Uuid::new_v4().simple());
        assert_eq!(
            db.create_community_with_owner(&host, &owner)
                .await
                .expect("create community call"),
            CreateCommunityWithOwnerResult::LimitReached
        );
        assert!(
            db.lookup_community_by_host(&host)
                .await
                .expect("look up rolled-back fresh host")
                .is_none(),
            "limit rejection must roll back the fresh community row"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_same_owner_create_returns_the_winning_row_to_both_callers() {
        let db = setup_db().await;
        let host = format!("concurrent-create-{}.example", Uuid::new_v4().simple());
        let owner = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let (first, second) = tokio::join!(
            db.create_community_with_owner(&host, owner),
            db.create_community_with_owner(&host, owner),
        );
        let first = first.expect("first concurrent create");
        let second = second.expect("second concurrent create");

        assert!(matches!(first, CreateCommunityWithOwnerResult::Created(_)));
        assert_eq!(first, second, "conflict loser re-reads the winning row");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn ensure_configured_community_reports_insert_winner() {
        let db = setup_db().await;
        let host = format!("ensure-community-{}.example", Uuid::new_v4().simple());

        let first = db
            .ensure_configured_community(&host)
            .await
            .expect("first ensure");
        assert!(first.created, "first ensure should report created");
        assert_eq!(first.host, host);

        let second = db
            .ensure_configured_community(&host)
            .await
            .expect("second ensure");
        assert!(!second.created, "second ensure should report existed");
        assert_eq!(second.id, first.id);
        assert_eq!(second.host, host);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn list_communities_owned_by_returns_only_owner_rows() {
        let db = setup_db().await;
        let community_a = CommunityId::from_uuid(make_community(&db.pool).await);
        let community_b = CommunityId::from_uuid(make_community(&db.pool).await);
        let community_c = CommunityId::from_uuid(make_community(&db.pool).await);
        // Unique per run: `list_communities_owned_by` is keyed only by pubkey,
        // so a shared fixed pubkey picks up communities leaked by sibling
        // ignored tests running against the same database.
        let owner = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let owner = owner.as_str();
        let other = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let other = other.as_str();

        db.bootstrap_owner(community_a, owner)
            .await
            .expect("owner A");
        db.bootstrap_owner(community_b, other)
            .await
            .expect("other owner B");
        db.add_relay_member(community_c, owner, "admin", None)
            .await
            .expect("admin C");

        let owned = db
            .list_communities_owned_by(owner)
            .await
            .expect("list owned communities");

        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, community_a);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn communities_of_channels_present_for_existing_absent_for_missing() {
        let db = setup_db().await;
        let community = make_community(&db.pool).await;
        let existing = Uuid::new_v4();
        insert_channel(&db.pool, community, existing).await;

        // Channel that is NOT inserted — the load-bearing case.
        let missing = Uuid::new_v4();

        let result = db
            .communities_of_channels(&[existing, missing])
            .await
            .expect("communities_of_channels");

        // (1) Existing channel → present with its true community.
        assert_eq!(
            result.get(&existing).copied(),
            Some(CommunityId::from_uuid(community)),
            "existing channel must map to its true community",
        );

        // (2) Missing channel → ABSENT from the map (never defaulted).
        // This is the contract the relay-side `MissingLookup → ImplBug`
        // fail-closed guard-rail depends on. If this assertion ever
        // weakens to `result.get(&missing) != Some(community)`, the
        // mutate-bite below stops biting.
        assert!(
            !result.contains_key(&missing),
            "missing channel must be absent from the result map, got {:?}",
            result.get(&missing),
        );

        // (3) Map size matches: exactly one entry, the existing one.
        assert_eq!(
            result.len(),
            1,
            "result map must contain only existing channels"
        );
    }
}
