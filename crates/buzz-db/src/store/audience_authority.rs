//! Current shared-session audience decisions, independent of discovery events.

use std::collections::HashSet;

use buzz_core::CommunityId;
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{observability, Db, DbError, Result};

/// Maximum source set evaluated at one authority snapshot.
pub const MAX_SOURCES: usize = 100;

/// A decision reveals no channel metadata or member identities.
#[derive(Debug, Serialize)]
pub struct AudienceDecision {
    /// The requested source ID, including for denied or nonexistent sources.
    pub id: Uuid,
    /// Whether this viewer may use this session for the destination audience.
    pub allowed: bool,
}

// Match buzz-app's sessionMetadata grammar, not a substring or a label hint.
fn session_description(description: Option<&str>) -> bool {
    let Some(description) = description else {
        return false;
    };
    let mut lines = description.split('\n');
    if lines.next() != Some("Buzz session (buzz.sessions/v1)") {
        return false;
    }
    let mut next = lines.next();
    if let Some(parent) = next.and_then(|line| line.strip_prefix("parent:")) {
        if !Uuid::parse_str(parent).is_ok_and(|id| id.to_string() == parent) {
            return false;
        }
        next = lines.next();
    }
    if next == Some("personal") {
        next = lines.next();
    }
    next.is_none()
}

impl Db {
    /// Evaluate current source identity, viewer access and audience containment.
    ///
    /// ONE writer SQL statement observes channels and all active membership rows
    /// at the same MVCC cut. Discovery events, replicas, cached rosters and capped
    /// member enumeration are never authorization inputs. Archived channels remain
    /// readable; deleted channels do not. Hidden DM placement is not read denial.
    /// This is an authorization-at-read decision, not a future publication lease.
    pub async fn audience_authority(
        &self,
        community: CommunityId,
        viewer: &nostr::PublicKey,
        destination: Uuid,
        sources: &[Uuid],
    ) -> Result<Vec<AudienceDecision>> {
        if sources.is_empty()
            || sources.len() > MAX_SOURCES
            || sources.iter().collect::<HashSet<_>>().len() != sources.len()
        {
            return Err(DbError::InvalidData("invalid audience source set".into()));
        }
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let rows = sqlx::query(
            r#"
            SELECT requested.id,
              CASE WHEN octet_length(s.description) <= 128 THEN s.description END AS description,
              COALESCE(
                s.id IS NOT NULL AND d.id IS NOT NULL
                AND s.channel_type = 'stream' AND s.visibility = 'private'
                AND d.visibility = 'private'
                AND EXISTS (SELECT 1 FROM channel_members v
                  WHERE v.community_id = $1 AND v.channel_id = s.id
                    AND v.pubkey = $2 AND v.removed_at IS NULL)
                AND EXISTS (SELECT 1 FROM channel_members v
                  WHERE v.community_id = $1 AND v.channel_id = d.id
                    AND v.pubkey = $2 AND v.removed_at IS NULL)
                AND NOT EXISTS (
                  SELECT 1 FROM channel_members audience
                  WHERE audience.community_id = $1 AND audience.channel_id = d.id
                    AND audience.removed_at IS NULL
                    AND NOT EXISTS (SELECT 1 FROM channel_members source_member
                      WHERE source_member.community_id = $1 AND source_member.channel_id = s.id
                        AND source_member.pubkey = audience.pubkey
                        AND source_member.removed_at IS NULL)
                ), false) AS allowed
            FROM unnest($4::uuid[]) WITH ORDINALITY requested(id, position)
            LEFT JOIN channels s ON s.community_id = $1 AND s.id = requested.id AND s.deleted_at IS NULL
            LEFT JOIN channels d ON d.community_id = $1 AND d.id = $3 AND d.deleted_at IS NULL
            ORDER BY requested.position
            "#,
        )
        .bind(community.as_uuid())
        .bind(viewer.to_bytes().to_vec())
        .bind(destination)
        .bind(sources)
        .fetch_all(&mut *conn)
        .await?;
        rows.into_iter()
            .map(|row| {
                let description: Option<String> = row.try_get("description")?;
                Ok(AudienceDecision {
                    id: row.try_get("id")?,
                    allowed: row.try_get::<bool, _>("allowed")?
                        && session_description(description.as_deref()),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_identity_matches_exact_description_grammar() {
        let header = "Buzz session (buzz.sessions/v1)";
        let parent = "12345678-1234-1234-1234-123456789abc";
        for suffix in [
            String::new(),
            "\npersonal".into(),
            format!("\nparent:{parent}"),
            format!("\nparent:{parent}\npersonal"),
        ] {
            assert!(session_description(Some(&format!("{header}{suffix}"))));
        }
        for suffix in [
            "\n",
            "\npersonal\npersonal",
            "\nparent:invalid",
            "\nparent:12345678123412341234123456789abc",
            "\nparent:12345678-1234-1234-1234-123456789ABC",
            "\npersonal\nparent:12345678-1234-1234-1234-123456789abc",
        ] {
            assert!(!session_description(Some(&format!("{header}{suffix}"))));
        }
        assert!(!session_description(None));
        assert!(!session_description(Some("ordinary channel")));
    }
}
