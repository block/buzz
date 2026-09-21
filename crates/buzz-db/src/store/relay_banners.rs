//! Deployment-global relay banner persistence and per-user state.
//!
//! A deployment has at most one active banner.  Delivery is community-scoped at
//! read time: an active banner either targets every community, including future
//! communities, or an explicit non-empty set of community ids.

use buzz_core::CommunityId;
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row as _, Transaction};
use std::collections::HashSet;
use uuid::Uuid;

use crate::{Db, Result};

/// Maximum relay banner message length in Unicode scalar count.
pub const MAX_BANNER_MESSAGE_CHARS: usize = 2_000;

/// Relay banner severity values accepted by the backend and emitted to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayBannerSeverity {
    /// Informational banner.
    Info,
    /// Warning banner.
    Warning,
    /// Urgent banner.
    Urgent,
}

impl RelayBannerSeverity {
    /// Returns the wire/database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Urgent => "urgent",
        }
    }

    fn parse(value: String) -> Result<Self> {
        match value.as_str() {
            "info" => Ok(Self::Info),
            "warning" => Ok(Self::Warning),
            "urgent" => Ok(Self::Urgent),
            other => Err(crate::DbError::InvalidData(format!(
                "invalid relay banner severity: {other}"
            ))),
        }
    }
}

/// Public targeting scope value used by the admin/client banner contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayBannerTargetScope {
    /// Banner targets every community.
    All,
    /// Banner targets selected communities only.
    Communities,
}

impl RelayBannerTargetScope {
    /// Returns the wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Communities => "communities",
        }
    }
}

/// Targeting scope for an operator-configured relay banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayBannerScope {
    /// Banner applies to every community, including communities created later.
    AllCommunities,
    /// Banner applies only to this non-empty community set.
    Communities(Vec<CommunityId>),
}

/// Input for replacing the deployment's active banner.
#[derive(Debug, Clone)]
pub struct RelayBannerUpsert {
    /// Severity/type of the banner.
    pub severity: RelayBannerSeverity,
    /// Plain-text message, 1..=2000 characters.
    pub message: String,
    /// Number of successful views each user may receive before suppression.
    pub max_displays: i32,
    /// Targeting scope.
    pub scope: RelayBannerScope,
    /// Authenticated operator pubkey performing the write.
    pub actor_pubkey: Vec<u8>,
}

/// Result of replacing the deployment's active banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayBannerUpsertOutcome {
    /// Previously active banner after being disabled, retaining its original target audience.
    pub previous: Option<RelayBannerRecord>,
    /// Newly active banner.
    pub active: RelayBannerRecord,
}

/// Stored relay banner plus targeting metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayBannerRecord {
    /// Monotonic internal banner id.
    pub id: i64,
    /// Stable public banner id exposed on the wire.
    pub public_id: Uuid,
    /// Severity/type of the banner.
    pub severity: RelayBannerSeverity,
    /// Plain-text message.
    pub message: String,
    /// Number of successful views each user may receive before suppression.
    pub max_displays: i32,
    /// Whether the banner targets every community.
    pub target_all_communities: bool,
    /// Explicit community targets when not targeting all communities.
    pub community_ids: Vec<CommunityId>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Row update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Operator pubkey that created the row.
    pub created_by: Vec<u8>,
    /// Disable timestamp, if disabled.
    pub disabled_at: Option<DateTime<Utc>>,
}

impl RelayBannerRecord {
    /// Returns the public target-scope enum for this banner.
    pub const fn target_scope(&self) -> RelayBannerTargetScope {
        if self.target_all_communities {
            RelayBannerTargetScope::All
        } else {
            RelayBannerTargetScope::Communities
        }
    }
}

/// Result of recording a banner view acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayBannerViewOutcome {
    /// The display was accepted and consumed one remaining view.
    Accepted {
        /// Display count after accepting this view.
        display_count: i32,
        /// True only when this acknowledgement consumed a new display.
        changed: bool,
    },
    /// The banner was already permanently dismissed by this user.
    Dismissed,
    /// The user had already exhausted `max_displays`.
    Exhausted {
        /// Display count already consumed by this user.
        display_count: i32,
    },
    /// The banner is not active or does not target the request community.
    NotEligible,
}

/// Result of recording a banner dismiss acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayBannerDismissOutcome {
    /// Dismiss state was present after the call. True means this call wrote it.
    Dismissed {
        /// Whether this call changed durable state from not-dismissed to dismissed.
        changed: bool,
    },
    /// The banner is not active or does not target the request community.
    NotEligible,
}

impl RelayBannerUpsert {
    fn validate(&self) -> Result<()> {
        let trimmed_empty = self.message.trim().is_empty();
        let char_count = self.message.chars().count();
        if trimmed_empty || char_count > MAX_BANNER_MESSAGE_CHARS {
            return Err(crate::DbError::InvalidData(format!(
                "relay banner message must be 1..={MAX_BANNER_MESSAGE_CHARS} characters"
            )));
        }
        if self.max_displays < 1 {
            return Err(crate::DbError::InvalidData(
                "relay banner max_displays must be >= 1".to_owned(),
            ));
        }
        if self.actor_pubkey.len() != 32 {
            return Err(crate::DbError::InvalidData(
                "relay banner actor pubkey must be 32 bytes".to_owned(),
            ));
        }
        if let RelayBannerScope::Communities(ids) = &self.scope {
            if ids.is_empty() {
                return Err(crate::DbError::InvalidData(
                    "relay banner community scope must be non-empty".to_owned(),
                ));
            }
            let mut seen = HashSet::with_capacity(ids.len());
            if ids.iter().any(|id| !seen.insert(*id)) {
                return Err(crate::DbError::InvalidData(
                    "relay banner community scope must not contain duplicates".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

async fn acquire_banner_lock(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('relay_banner_active', 0))")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn next_banner_protocol_timestamp(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<DateTime<Utc>> {
    let timestamp = sqlx::query_scalar::<_, DateTime<Utc>>(
        r#"
        SELECT to_timestamp(GREATEST(
            EXTRACT(EPOCH FROM date_trunc('second', now()))::bigint,
            COALESCE((SELECT MAX(FLOOR(EXTRACT(EPOCH FROM updated_at))::bigint) + 1 FROM relay_banners), 0)
        ))::timestamptz
        "#,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(timestamp)
}

async fn active_banner_in_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<Option<RelayBannerRecord>> {
    let row = sqlx::query(
        r#"
        SELECT
            b.id,
            b.public_id,
            b.severity,
            b.message,
            b.max_displays,
            b.target_all_communities,
            b.created_by,
            b.created_at,
            b.updated_at,
            b.disabled_at,
            COALESCE(array_agg(bc.community_id ORDER BY bc.community_id)
                FILTER (WHERE bc.community_id IS NOT NULL), ARRAY[]::uuid[]) AS community_ids
        FROM relay_banners b
        LEFT JOIN relay_banner_communities bc ON bc.banner_id = b.id
        WHERE b.disabled_at IS NULL
        GROUP BY b.id
        ORDER BY b.id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(row_to_banner).transpose()
}

async fn disable_active_banner_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    active: &mut RelayBannerRecord,
    actor_pubkey: &[u8],
) -> Result<()> {
    let protocol_timestamp = next_banner_protocol_timestamp(tx).await?;
    let updated = sqlx::query(
        r#"
        UPDATE relay_banners
        SET disabled_at = $1, disabled_by = $2, updated_at = $1, updated_by = $2
        WHERE id = $3 AND disabled_at IS NULL
        RETURNING updated_at, disabled_at
        "#,
    )
    .bind(protocol_timestamp)
    .bind(actor_pubkey)
    .bind(active.id)
    .fetch_one(&mut **tx)
    .await?;
    active.updated_at = updated.try_get("updated_at")?;
    active.disabled_at = updated.try_get("disabled_at")?;
    Ok(())
}

async fn banner_internal_id_for_public_id(
    tx: &mut Transaction<'_, Postgres>,
    public_id: Uuid,
    require_active: bool,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM relay_banners
        WHERE public_id = $1
          AND (NOT $2 OR disabled_at IS NULL)
        "#,
    )
    .bind(public_id)
    .bind(require_active)
    .fetch_optional(&mut **tx)
    .await
    .map_err(Into::into)
}

async fn banner_targets_community(
    tx: &mut Transaction<'_, Postgres>,
    banner_id: i64,
    community_id: CommunityId,
) -> Result<bool> {
    let eligible = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM relay_banners b
            WHERE b.id = $1
              AND b.disabled_at IS NULL
              AND (
                    b.target_all_communities
                 OR EXISTS (
                        SELECT 1
                        FROM relay_banner_communities bc
                        WHERE bc.banner_id = b.id
                          AND bc.community_id = $2
                    )
              )
        )
        "#,
    )
    .bind(banner_id)
    .bind(community_id.as_uuid())
    .fetch_one(&mut **tx)
    .await?;
    Ok(eligible)
}

async fn relay_banner_user_eligible_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    banner_id: i64,
    community_id: CommunityId,
    pubkey: &[u8],
    require_active: bool,
) -> Result<bool> {
    let eligible = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM relay_banners b
            LEFT JOIN relay_banner_user_state us
              ON us.banner_id = b.id
             AND us.community_id = $2
             AND us.pubkey = $3
            WHERE b.id = $1
              AND (NOT $4 OR b.disabled_at IS NULL)
              AND (
                    b.target_all_communities
                 OR EXISTS (
                        SELECT 1
                        FROM relay_banner_communities bc
                        WHERE bc.banner_id = b.id
                          AND bc.community_id = $2
                    )
              )
              AND us.dismissed_at IS NULL
              AND (NOT $4 OR COALESCE(us.display_count, 0) < b.max_displays)
        )
        "#,
    )
    .bind(banner_id)
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .bind(require_active)
    .fetch_one(&mut **tx)
    .await?;
    Ok(eligible)
}

impl Db {
    /// Lists active communities for the admin banner community picker.
    #[datastore_span(name = "admin_list_banner_communities", system = "postgresql")]
    pub async fn admin_list_banner_communities(&self) -> Result<Vec<crate::CommunityRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT id, host
            FROM communities
            WHERE archived_at IS NULL
              AND deleted_at IS NULL
              AND deletion_state = 'active'
            ORDER BY lower(host), id
            "#,
        )
        .fetch_all(&mut *connection)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(crate::CommunityRecord {
                    id: CommunityId::from_uuid(row.try_get("id")?),
                    host: row.try_get("host")?,
                })
            })
            .collect()
    }

    /// Lists relay banners, newest first, retaining disabled history.
    #[datastore_span(name = "admin_list_relay_banners", system = "postgresql")]
    pub async fn admin_list_relay_banners(&self, limit: i64) -> Result<Vec<RelayBannerRecord>> {
        let limit = limit.clamp(1, 200);
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT
                b.id,
                b.public_id,
                b.severity,
                b.message,
                b.max_displays,
                b.target_all_communities,
                b.created_by,
                b.created_at,
                b.updated_at,
                b.disabled_at,
                COALESCE(array_agg(bc.community_id ORDER BY bc.community_id)
                    FILTER (WHERE bc.community_id IS NOT NULL), ARRAY[]::uuid[]) AS community_ids
            FROM relay_banners b
            LEFT JOIN relay_banner_communities bc ON bc.banner_id = b.id
            GROUP BY b.id
            ORDER BY b.updated_at DESC, b.id DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
        rows.into_iter().map(row_to_banner).collect()
    }

    /// Returns a banner by its public id, including disabled history.
    #[datastore_span(name = "relay_banner_by_public_id", system = "postgresql")]
    pub async fn relay_banner_by_public_id(
        &self,
        public_id: Uuid,
    ) -> Result<Option<RelayBannerRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT
                b.id,
                b.public_id,
                b.severity,
                b.message,
                b.max_displays,
                b.target_all_communities,
                b.created_by,
                b.created_at,
                b.updated_at,
                b.disabled_at,
                COALESCE(array_agg(bc.community_id ORDER BY bc.community_id)
                    FILTER (WHERE bc.community_id IS NOT NULL), ARRAY[]::uuid[]) AS community_ids
            FROM relay_banners b
            LEFT JOIN relay_banner_communities bc ON bc.banner_id = b.id
            WHERE b.public_id = $1
            GROUP BY b.id
            "#,
        )
        .bind(public_id)
        .fetch_optional(&mut *connection)
        .await?;
        row.map(row_to_banner).transpose()
    }

    /// Returns the currently active deployment banner, if any.
    #[datastore_span(name = "admin_get_active_relay_banner", system = "postgresql")]
    pub async fn admin_get_active_relay_banner(&self) -> Result<Option<RelayBannerRecord>> {
        let mut items = self.admin_list_active_relay_banners(1).await?;
        Ok(items.pop())
    }

    async fn admin_list_active_relay_banners(&self, limit: i64) -> Result<Vec<RelayBannerRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT
                b.id,
                b.public_id,
                b.severity,
                b.message,
                b.max_displays,
                b.target_all_communities,
                b.created_by,
                b.created_at,
                b.updated_at,
                b.disabled_at,
                COALESCE(array_agg(bc.community_id ORDER BY bc.community_id)
                    FILTER (WHERE bc.community_id IS NOT NULL), ARRAY[]::uuid[]) AS community_ids
            FROM relay_banners b
            LEFT JOIN relay_banner_communities bc ON bc.banner_id = b.id
            WHERE b.disabled_at IS NULL
            GROUP BY b.id
            ORDER BY b.id DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?;
        rows.into_iter().map(row_to_banner).collect()
    }

    /// Replaces the single active deployment banner and returns the new row.
    #[datastore_span(name = "admin_upsert_relay_banner", system = "postgresql")]
    pub async fn admin_upsert_relay_banner(
        &self,
        input: RelayBannerUpsert,
    ) -> Result<RelayBannerUpsertOutcome> {
        input.validate()?;
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        acquire_banner_lock(&mut tx).await?;

        let mut previous = active_banner_in_tx(&mut tx).await?;
        if let Some(active) = previous.as_mut() {
            disable_active_banner_in_tx(&mut tx, active, &input.actor_pubkey).await?;
        }

        let protocol_timestamp = next_banner_protocol_timestamp(&mut tx).await?;
        let target_all = matches!(input.scope, RelayBannerScope::AllCommunities);
        let row = sqlx::query(
            r#"
            INSERT INTO relay_banners
                (severity, message, max_displays, target_all_communities, created_at, updated_at, created_by, updated_by)
            VALUES ($1, $2, $3, $4, $5, $5, $6, $6)
            RETURNING id, public_id, severity, message, max_displays, target_all_communities,
                      created_by, created_at, updated_at, disabled_at
            "#,
        )
        .bind(input.severity.as_str())
        .bind(&input.message)
        .bind(input.max_displays)
        .bind(target_all)
        .bind(protocol_timestamp)
        .bind(&input.actor_pubkey)
        .fetch_one(&mut *tx)
        .await?;
        let banner_id: i64 = row.try_get("id")?;

        let community_ids = match input.scope {
            RelayBannerScope::AllCommunities => Vec::new(),
            RelayBannerScope::Communities(ids) => {
                for community_id in &ids {
                    sqlx::query(
                        "INSERT INTO relay_banner_communities (banner_id, community_id) VALUES ($1, $2)",
                    )
                    .bind(banner_id)
                    .bind(community_id.as_uuid())
                    .execute(&mut *tx)
                    .await?;
                }
                ids
            }
        };

        tx.commit().await?;
        Ok(RelayBannerUpsertOutcome {
            previous,
            active: RelayBannerRecord {
                id: banner_id,
                public_id: row.try_get("public_id")?,
                severity: RelayBannerSeverity::parse(row.try_get("severity")?)?,
                message: row.try_get("message")?,
                max_displays: row.try_get("max_displays")?,
                target_all_communities: row.try_get("target_all_communities")?,
                community_ids,
                created_by: row.try_get("created_by")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                disabled_at: row.try_get("disabled_at")?,
            },
        })
    }

    /// Disables the currently active deployment banner, preserving history and state.
    #[datastore_span(name = "admin_disable_active_relay_banner", system = "postgresql")]
    pub async fn admin_disable_active_relay_banner(
        &self,
        actor_pubkey: &[u8],
    ) -> Result<Option<RelayBannerRecord>> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        acquire_banner_lock(&mut tx).await?;
        let Some(mut active) = active_banner_in_tx(&mut tx).await? else {
            tx.commit().await?;
            return Ok(None);
        };
        disable_active_banner_in_tx(&mut tx, &mut active, actor_pubkey).await?;
        tx.commit().await?;
        Ok(Some(active))
    }

    /// Returns whether this user would receive the banner in this community.
    #[datastore_span(name = "relay_banner_user_eligible", system = "postgresql")]
    pub async fn relay_banner_user_eligible(
        &self,
        banner_id: i64,
        community_id: CommunityId,
        pubkey: &[u8],
        require_active: bool,
    ) -> Result<bool> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        let eligible = relay_banner_user_eligible_in_tx(
            &mut tx,
            banner_id,
            community_id,
            pubkey,
            require_active,
        )
        .await?;
        tx.commit().await?;
        Ok(eligible)
    }

    /// Resolves the active banner eligible for this user in this community.
    #[datastore_span(name = "active_relay_banner_for_user", system = "postgresql")]
    pub async fn active_relay_banner_for_user(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<RelayBannerRecord>> {
        let mut connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let row = sqlx::query(
            r#"
            SELECT
                b.id,
                b.public_id,
                b.severity,
                b.message,
                b.max_displays,
                b.target_all_communities,
                b.created_by,
                b.created_at,
                b.updated_at,
                b.disabled_at,
                COALESCE(array_agg(bc_all.community_id ORDER BY bc_all.community_id)
                    FILTER (WHERE bc_all.community_id IS NOT NULL), ARRAY[]::uuid[]) AS community_ids
            FROM relay_banners b
            LEFT JOIN relay_banner_communities bc_all ON bc_all.banner_id = b.id
            LEFT JOIN relay_banner_user_state us
              ON us.banner_id = b.id
             AND us.community_id = $1
             AND us.pubkey = $2
            WHERE b.disabled_at IS NULL
              AND (b.target_all_communities OR EXISTS (
                    SELECT 1
                    FROM relay_banner_communities bc
                    WHERE bc.banner_id = b.id
                      AND bc.community_id = $1
              ))
              AND us.dismissed_at IS NULL
              AND COALESCE(us.display_count, 0) < b.max_displays
            GROUP BY b.id, us.display_count, us.dismissed_at
            ORDER BY b.id DESC
            LIMIT 1
            "#,
        )
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .fetch_optional(&mut *connection)
        .await?;
        row.map(row_to_banner).transpose()
    }

    /// Records a successful client render/view acknowledgement.
    #[datastore_span(name = "ack_relay_banner_view", system = "postgresql")]
    pub async fn ack_relay_banner_view(
        &self,
        community_id: CommunityId,
        public_id: Uuid,
        pubkey: &[u8],
        view_id: Uuid,
    ) -> Result<RelayBannerViewOutcome> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        let Some(banner_id) = banner_internal_id_for_public_id(&mut tx, public_id, true).await?
        else {
            tx.rollback().await?;
            return Ok(RelayBannerViewOutcome::NotEligible);
        };
        if !banner_targets_community(&mut tx, banner_id, community_id).await? {
            tx.rollback().await?;
            return Ok(RelayBannerViewOutcome::NotEligible);
        }

        let inserted_view = sqlx::query_scalar::<_, bool>(
            r#"
            INSERT INTO relay_banner_view_acks
                (banner_id, community_id, pubkey, view_id)
            SELECT $1, $2, $3, $4
            WHERE EXISTS (
                SELECT 1
                FROM relay_banners b
                WHERE b.id = $1
                  AND b.disabled_at IS NULL
                  AND (b.target_all_communities OR EXISTS (
                        SELECT 1
                        FROM relay_banner_communities bc
                        WHERE bc.banner_id = b.id AND bc.community_id = $2
                  ))
            )
            ON CONFLICT (banner_id, community_id, pubkey, view_id) DO NOTHING
            RETURNING TRUE
            "#,
        )
        .bind(banner_id)
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .bind(view_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);

        if !inserted_view {
            let state = sqlx::query(
                r#"
                SELECT us.display_count, us.dismissed_at IS NOT NULL AS dismissed
                FROM relay_banner_user_state us
                WHERE us.banner_id = $1 AND us.community_id = $2 AND us.pubkey = $3
                "#,
            )
            .bind(banner_id)
            .bind(community_id.as_uuid())
            .bind(pubkey)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(match state {
                Some(row) if row.try_get::<bool, _>("dismissed")? => {
                    RelayBannerViewOutcome::Dismissed
                }
                Some(row) => RelayBannerViewOutcome::Accepted {
                    display_count: row.try_get("display_count")?,
                    changed: false,
                },
                None => RelayBannerViewOutcome::NotEligible,
            });
        }

        let row = sqlx::query(
            r#"
            INSERT INTO relay_banner_user_state
                (banner_id, community_id, pubkey, display_count, first_viewed_at, last_viewed_at)
            SELECT b.id, $2, $3, 1, now(), now()
            FROM relay_banners b
            WHERE b.id = $1
              AND b.disabled_at IS NULL
              AND b.max_displays >= 1
            ON CONFLICT (banner_id, community_id, pubkey) DO UPDATE SET
                display_count = relay_banner_user_state.display_count + 1,
                first_viewed_at = COALESCE(relay_banner_user_state.first_viewed_at, now()),
                last_viewed_at = now(),
                updated_at = now()
            WHERE relay_banner_user_state.dismissed_at IS NULL
              AND relay_banner_user_state.display_count < (
                    SELECT max_displays FROM relay_banners WHERE id = $1 AND disabled_at IS NULL
              )
            RETURNING display_count
            "#,
        )
        .bind(banner_id)
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .fetch_optional(&mut *tx)
        .await?;

        let outcome = if let Some(row) = row {
            RelayBannerViewOutcome::Accepted {
                display_count: row.try_get("display_count")?,
                changed: true,
            }
        } else {
            let state = sqlx::query(
                r#"
                SELECT us.display_count, us.dismissed_at IS NOT NULL AS dismissed
                FROM relay_banner_user_state us
                WHERE us.banner_id = $1 AND us.community_id = $2 AND us.pubkey = $3
                "#,
            )
            .bind(banner_id)
            .bind(community_id.as_uuid())
            .bind(pubkey)
            .fetch_optional(&mut *tx)
            .await?;
            match state {
                Some(row) if row.try_get::<bool, _>("dismissed")? => {
                    RelayBannerViewOutcome::Dismissed
                }
                Some(row) => RelayBannerViewOutcome::Exhausted {
                    display_count: row.try_get("display_count")?,
                },
                None => RelayBannerViewOutcome::NotEligible,
            }
        };
        if !matches!(
            outcome,
            RelayBannerViewOutcome::Accepted { changed: true, .. }
        ) {
            tx.rollback().await?;
            return Ok(outcome);
        }
        tx.commit().await?;
        Ok(outcome)
    }

    /// Permanently dismisses a banner for a user in one community.
    #[datastore_span(name = "ack_relay_banner_dismiss", system = "postgresql")]
    pub async fn ack_relay_banner_dismiss(
        &self,
        community_id: CommunityId,
        public_id: Uuid,
        pubkey: &[u8],
    ) -> Result<RelayBannerDismissOutcome> {
        let connection = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::Authorization,
        )
        .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        let Some(banner_id) = banner_internal_id_for_public_id(&mut tx, public_id, true).await?
        else {
            tx.rollback().await?;
            return Ok(RelayBannerDismissOutcome::NotEligible);
        };
        if !banner_targets_community(&mut tx, banner_id, community_id).await? {
            tx.rollback().await?;
            return Ok(RelayBannerDismissOutcome::NotEligible);
        }

        let changed = sqlx::query_scalar::<_, bool>(
            r#"
            WITH inserted AS (
                INSERT INTO relay_banner_user_state
                    (banner_id, community_id, pubkey, dismissed_at)
                VALUES ($1, $2, $3, now())
                ON CONFLICT (banner_id, community_id, pubkey) DO NOTHING
                RETURNING TRUE AS changed
            ), updated AS (
                UPDATE relay_banner_user_state
                SET dismissed_at = now(), updated_at = now()
                WHERE banner_id = $1
                  AND community_id = $2
                  AND pubkey = $3
                  AND dismissed_at IS NULL
                  AND NOT EXISTS (SELECT 1 FROM inserted)
                RETURNING TRUE AS changed
            )
            SELECT COALESCE(
                (SELECT changed FROM inserted),
                (SELECT changed FROM updated),
                FALSE
            )
            "#,
        )
        .bind(banner_id)
        .bind(community_id.as_uuid())
        .bind(pubkey)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RelayBannerDismissOutcome::Dismissed { changed })
    }
}

fn row_to_banner(row: sqlx::postgres::PgRow) -> Result<RelayBannerRecord> {
    let community_uuids: Vec<Uuid> = row.try_get("community_ids")?;
    Ok(RelayBannerRecord {
        id: row.try_get("id")?,
        public_id: row.try_get("public_id")?,
        severity: RelayBannerSeverity::parse(row.try_get("severity")?)?,
        message: row.try_get("message")?,
        max_displays: row.try_get("max_displays")?,
        target_all_communities: row.try_get("target_all_communities")?,
        community_ids: community_uuids
            .into_iter()
            .map(CommunityId::from_uuid)
            .collect(),
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        disabled_at: row.try_get("disabled_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    static RELAY_BANNER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn setup_db() -> (tokio::sync::MutexGuard<'static, ()>, Db) {
        let guard = RELAY_BANNER_TEST_LOCK.lock().await;
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        (guard, Db::from_pool(pool))
    }

    pub(crate) async fn insert_test_community(pool: &PgPool, host_prefix: &str) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("{host_prefix}-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        insert_test_community(pool, "banner").await
    }

    fn actor(byte: u8) -> Vec<u8> {
        vec![byte; 32]
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn upsert_replaces_single_active_banner_and_retains_history() {
        let (_guard, db) = setup_db().await;
        let first = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "first".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(1),
            })
            .await
            .expect("insert first banner")
            .active;
        let second = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Warning,
                message: "second".to_owned(),
                max_displays: 2,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(2),
            })
            .await
            .expect("replace banner")
            .active;

        let active = db
            .admin_get_active_relay_banner()
            .await
            .expect("get active")
            .expect("active banner");
        assert_eq!(active.id, second.id);
        let all = db
            .admin_list_relay_banners(200)
            .await
            .expect("list banners");
        let created: Vec<_> = all
            .iter()
            .filter(|banner| [first.public_id, second.public_id].contains(&banner.public_id))
            .collect();
        assert_eq!(created.len(), 2);
        assert_eq!(
            created
                .iter()
                .map(|banner| banner.public_id)
                .collect::<Vec<_>>(),
            vec![second.public_id, first.public_id],
            "history list should return only this test's created banners in newest-first order"
        );
        assert!(created
            .iter()
            .any(|b| b.id == first.id && b.disabled_at.is_some()));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn rapid_replacement_and_disable_use_monotonic_protocol_seconds() {
        let (_guard, db) = setup_db().await;
        let first = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "first monotonic".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(18),
            })
            .await
            .expect("insert first banner")
            .active;
        let second = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Warning,
                message: "second monotonic".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(18),
            })
            .await
            .expect("replace banner");
        let disabled_first = second.previous.as_ref().expect("disabled first");
        assert_eq!(disabled_first.id, first.id);
        assert!(
            disabled_first.updated_at.timestamp() < second.active.updated_at.timestamp(),
            "disabled clear must sort before replacement active at protocol-second precision"
        );

        let disabled_second = db
            .admin_disable_active_relay_banner(&actor(18))
            .await
            .expect("disable active banner")
            .expect("disabled second");
        assert_eq!(disabled_second.id, second.active.id);
        assert!(
            second.active.updated_at.timestamp() < disabled_second.updated_at.timestamp(),
            "final disable must sort after the replacement active at protocol-second precision"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn community_scope_filters_delivery() {
        let (_guard, db) = setup_db().await;
        let included = make_community(&db.pool).await;
        let excluded = make_community(&db.pool).await;
        let user = actor(3);
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Urgent,
                message: "scoped".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::Communities(vec![included]),
                actor_pubkey: actor(4),
            })
            .await
            .expect("insert scoped banner")
            .active;

        assert_eq!(banner.community_ids, vec![included]);
        assert!(db
            .active_relay_banner_for_user(included, &user)
            .await
            .expect("included lookup")
            .is_some());
        assert!(db
            .active_relay_banner_for_user(excluded, &user)
            .await
            .expect("excluded lookup")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn duplicate_community_scope_is_rejected() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let result = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "duplicates".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::Communities(vec![community, community]),
                actor_pubkey: actor(15),
            })
            .await;

        assert!(
            matches!(result, Err(crate::DbError::InvalidData(message)) if message.contains("duplicates"))
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn disabled_banner_remains_eligible_when_active_requirement_is_lifted() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(16);
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Warning,
                message: "disable eligibility".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(17),
            })
            .await
            .expect("insert banner")
            .active;
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("exhaust display"),
            RelayBannerViewOutcome::Accepted {
                display_count: 1,
                changed: true,
            }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("already exhausted"),
            RelayBannerViewOutcome::Exhausted { display_count: 1 }
        );
        let disabled = db
            .admin_disable_active_relay_banner(&actor(17))
            .await
            .expect("disable banner")
            .expect("active banner");

        assert_eq!(disabled.id, banner.id);
        assert!(disabled.disabled_at.is_some());
        assert!(db
            .relay_banner_user_eligible(banner.id, community, &user, false)
            .await
            .expect("pre-disable eligibility lookup"));
        assert!(!db
            .relay_banner_user_eligible(banner.id, community, &user, true)
            .await
            .expect("active eligibility lookup"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn view_ack_consumes_display_and_then_exhausts() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(5);
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "limited".to_owned(),
                max_displays: 1,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(6),
            })
            .await
            .expect("insert banner")
            .active;

        assert_eq!(
            db.active_relay_banner_for_user(community, &user)
                .await
                .expect("lookup before view")
                .as_ref()
                .map(|active| active.id),
            Some(banner.id),
            "delivery lookup must not consume max_displays before client view ack"
        );
        assert_eq!(
            db.active_relay_banner_for_user(community, &user)
                .await
                .expect("second lookup before view")
                .as_ref()
                .map(|active| active.id),
            Some(banner.id),
            "repeated delivery lookups without view ack must not exhaust max_displays"
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("first view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 1,
                changed: true
            }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("second view"),
            RelayBannerViewOutcome::Exhausted { display_count: 1 }
        );
        assert!(db
            .active_relay_banner_for_user(community, &user)
            .await
            .expect("post-exhaust lookup")
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn view_ack_same_key_retry_does_not_consume_again() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(9);
        let view_id = Uuid::new_v4();
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "retry".to_owned(),
                max_displays: 2,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(10),
            })
            .await
            .expect("insert banner")
            .active;

        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, view_id)
                .await
                .expect("first view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 1,
                changed: true
            }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, view_id)
                .await
                .expect("retry view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 1,
                changed: false
            }
        );
        assert!(db
            .active_relay_banner_for_user(community, &user)
            .await
            .expect("still eligible after duplicate")
            .is_some());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn distinct_view_keys_increment_until_cap() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(11);
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Warning,
                message: "cap".to_owned(),
                max_displays: 2,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(12),
            })
            .await
            .expect("insert banner")
            .active;

        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("first view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 1,
                changed: true
            }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("second view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 2,
                changed: true
            }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("third view"),
            RelayBannerViewOutcome::Exhausted { display_count: 2 }
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_same_view_key_consumes_once() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(13);
        let view_id = Uuid::new_v4();
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Urgent,
                message: "concurrent".to_owned(),
                max_displays: 2,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(14),
            })
            .await
            .expect("insert banner")
            .active;

        let (first, second) = tokio::join!(
            db.ack_relay_banner_view(community, banner.public_id, &user, view_id),
            db.ack_relay_banner_view(community, banner.public_id, &user, view_id),
        );
        let changed = [first.expect("first"), second.expect("second")]
            .into_iter()
            .filter(|outcome| {
                matches!(
                    outcome,
                    RelayBannerViewOutcome::Accepted { changed: true, .. }
                )
            })
            .count();
        assert_eq!(changed, 1);
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("second distinct view"),
            RelayBannerViewOutcome::Accepted {
                display_count: 2,
                changed: true
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn dismiss_permanently_suppresses_banner() {
        let (_guard, db) = setup_db().await;
        let community = make_community(&db.pool).await;
        let user = actor(7);
        let banner = db
            .admin_upsert_relay_banner(RelayBannerUpsert {
                severity: RelayBannerSeverity::Info,
                message: "dismiss me".to_owned(),
                max_displays: 3,
                scope: RelayBannerScope::AllCommunities,
                actor_pubkey: actor(8),
            })
            .await
            .expect("insert banner")
            .active;

        assert_eq!(
            db.ack_relay_banner_dismiss(community, banner.public_id, &user)
                .await
                .expect("dismiss"),
            RelayBannerDismissOutcome::Dismissed { changed: true }
        );
        assert_eq!(
            db.ack_relay_banner_dismiss(community, banner.public_id, &user)
                .await
                .expect("second dismiss"),
            RelayBannerDismissOutcome::Dismissed { changed: false }
        );
        assert_eq!(
            db.ack_relay_banner_view(community, banner.public_id, &user, Uuid::new_v4())
                .await
                .expect("view after dismiss"),
            RelayBannerViewOutcome::Dismissed
        );
        assert!(db
            .active_relay_banner_for_user(community, &user)
            .await
            .expect("lookup after dismiss")
            .is_none());
    }
}
