//! Bounded, primary-only projections of the viewer's canonical memberships.

use buzz_core::CommunityId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// Maximum memberships in a complete NIP-CL version-1 response.
pub const MAX_CHANNELS: usize = 4096;
/// Maximum encoded response bytes, including the signed event and envelope.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Types understood by version 1, in canonical wire order.
pub const CHANNEL_TYPES: &[&str] = &["dm", "forum", "stream", "system", "workflow"];

/// Exactly the fields needed to establish the viewer's membership.
/// Field declaration order is part of the snapshot content-identity encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewerMembership {
    /// Canonical channel UUID.
    pub id: Uuid,
    /// This viewer's role; never a list of other members.
    pub role: String,
    /// Channel type, within the explicitly requested scope.
    pub channel_type: String,
    /// Current channel visibility, independently of membership.
    pub visibility: String,
    /// Archived channels remain memberships, with read-only behavior.
    pub archived: bool,
}

/// A complete projection at one primary statement's MVCC cut.
#[derive(Debug)]
pub struct MembershipSnapshot {
    /// Content identity, never an ordered revision or a live-delivery cursor.
    pub snapshot_id: String,
    /// Every active membership in the requested types, sorted by channel UUID.
    pub channels: Vec<ViewerMembership>,
}

// No events, rosters, profiles, other-member aggregates or replica routing.
// Probe one additional row; never silently certify a capped set as complete.
// Keep the lookup dependent on the viewer row to avoid a community-wide scan.
// Probe two rows so an accidental loss of the tenant/key predicate cannot hide
// a duplicate behind LIMIT 1: strict ID validation below then fails closed.
// The outer probe counts AFTER type/deletion filters.
const SNAPSHOT_SQL: &str = "SELECT cm.channel_id AS id, cm.role::text AS role,
    c.channel_type, c.visibility, c.archived
    FROM channel_members cm
    CROSS JOIN LATERAL (
        SELECT c.channel_type::text AS channel_type, c.visibility::text AS visibility,
               c.archived_at IS NOT NULL AS archived
        FROM channels c
        WHERE c.community_id = cm.community_id AND c.id = cm.channel_id
          AND (NOT $5 OR (c.visibility = 'open' AND NOT c.unlisted))
          AND c.deleted_at IS NULL AND c.channel_type = ANY($3::text[]::channel_type[])
        LIMIT 2
    ) c
    WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
    ORDER BY cm.channel_id LIMIT $4";

/// Validate the explicit type scope without silently dropping unknown types.
pub fn valid_types(types: &[String]) -> bool {
    !types.is_empty()
        && types.len() <= CHANNEL_TYPES.len()
        && types.iter().all(|t| CHANNEL_TYPES.contains(&t.as_str()))
        && types.windows(2).all(|pair| pair[0] < pair[1])
}

/// Public mode accepts only listed channel types, never DMs or system channels.
pub fn valid_public_types(types: &[String]) -> bool {
    valid_types(types)
        && types
            .iter()
            .all(|t| ["forum", "stream", "workflow"].contains(&t.as_str()))
}

impl Db {
    /// Project only the viewer's active memberships from one primary MVCC cut.
    ///
    /// Archive/visibility/role/deletion changes share the same statement cut.
    /// Hidden DMs are deliberately included; NIP-DV owns hiding separately.
    /// No full-roster or per-user projection cache is consulted or maintained.
    pub async fn membership_snapshot(
        &self,
        community: CommunityId,
        viewer: &nostr::PublicKey,
        types: &[String],
    ) -> Result<MembershipSnapshot> {
        self.project_memberships(community, viewer, types, false)
            .await
    }

    /// Project a subject's listed open memberships at one primary statement cut.
    pub async fn public_memberships(
        &self,
        community: CommunityId,
        subject: &nostr::PublicKey,
        types: &[String],
    ) -> Result<MembershipSnapshot> {
        self.project_memberships(community, subject, types, true)
            .await
    }

    async fn project_memberships(
        &self,
        community: CommunityId,
        viewer: &nostr::PublicKey,
        types: &[String],
        public: bool,
    ) -> Result<MembershipSnapshot> {
        if !valid_types(types) || (public && !valid_public_types(types)) {
            return Err(DbError::InvalidData("invalid membership type scope".into()));
        }
        let started = std::time::Instant::now();
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::Authorization,
        )
        .await?;
        let rows = sqlx::query(SNAPSHOT_SQL)
            .bind(community.as_uuid())
            .bind(viewer.to_bytes().to_vec())
            .bind(types)
            .bind((MAX_CHANNELS + 1) as i64)
            .bind(public)
            .fetch_all(&mut *conn)
            .await?;
        metrics::histogram!("buzz_membership_snapshot_query_seconds", "mode" => if public {"membership_public"} else {"membership_snapshot"})
            .record(started.elapsed().as_secs_f64());
        if rows.len() > MAX_CHANNELS {
            return Err(DbError::MembershipSnapshotTooLarge);
        }
        let channels = rows
            .into_iter()
            .map(|row| {
                let entry = ViewerMembership {
                    id: row.try_get("id")?,
                    role: row.try_get("role")?,
                    channel_type: row.try_get("channel_type")?,
                    visibility: row.try_get("visibility")?,
                    archived: row.try_get("archived")?,
                };
                if !["owner", "admin", "member", "bot", "guest"].contains(&entry.role.as_str())
                    || !types.contains(&entry.channel_type)
                    || !["open", "private"].contains(&entry.visibility.as_str())
                    || (public && entry.visibility != "open")
                {
                    return Err(DbError::InvalidData(
                        "invalid membership projection row".into(),
                    ));
                }
                Ok(entry)
            })
            .collect::<Result<Vec<_>>>()?;
        if channels.windows(2).any(|pair| pair[0].id >= pair[1].id) {
            return Err(DbError::InvalidData(
                "membership projection IDs are not unique and ordered".into(),
            ));
        }
        let identity = serde_json::to_vec(&(
            if public {
                "nip-cl-public-membership-v1"
            } else {
                "nip-cl-membership-state-v1"
            },
            community.as_uuid(),
            viewer.to_hex(),
            types,
            &channels,
        ))?;
        Ok(MembershipSnapshot {
            snapshot_id: hex::encode(Sha256::digest(identity)),
            channels,
        })
    }
}

#[cfg(test)]
mod postgres_tests;
