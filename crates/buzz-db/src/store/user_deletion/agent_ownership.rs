//! One ownership rule for every user-deletion handler, independent of membership.
use super::normalize_user_deletion_pubkey;
use crate::{DbError, Result};
use sqlx::{PgConnection, Postgres, QueryBuilder};
use uuid::Uuid;

pub(super) const MAX_TARGETS: usize = 10_000;

/// Preserve the caller's existing bound: agent rows, or identities including the human.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DiscoveryLimit {
    AgentRows,
    TargetKeys,
}

/// Fresh human and direct-agent identities, retaining each agent's community.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct UserDeletionTargets {
    /// The human remains the global discovery root.
    pub human: String,
    /// Agent authority applies only in the recorded community.
    pub agents: Vec<(Uuid, String)>,
}

impl UserDeletionTargets {
    /// Sorted distinct identities. Global callers must retain community pairs
    /// when authorizing deletion; this flattened list alone grants no authority.
    pub fn pubkeys(&self) -> Vec<String> {
        let mut keys = vec![self.human.clone()];
        keys.extend(self.agents.iter().map(|(_, key)| key.clone()));
        keys.sort();
        keys.dedup();
        keys
    }
}

/// Read current direct ownership in the caller's transaction. No membership or
/// active-profile filter: deactivated profiles remain evidence until cleanup ends.
/// Missing profiles grant no agent authority, and agent ownership is not recursive.
/// `lock` retains FOR SHARE locks until the caller ends its transaction; callers
/// still own write admission and the offline/stopped-writer operating contract.
/// With `community: None` and `DiscoveryLimit::TargetKeys`, callers must pass
/// `lock: false`: this query groups profiles by agent, and PostgreSQL does not
/// allow FOR SHARE with GROUP BY. Use community-scoped discovery for row locking.
pub(super) async fn discover(
    conn: &mut PgConnection,
    human: &str,
    community: Option<Uuid>,
    lock: bool,
    limit: DiscoveryLimit,
) -> Result<UserDeletionTargets> {
    let human = normalize_user_deletion_pubkey(human)?;
    // Media historically limits distinct identities, not community profiles.
    // Group those profiles before LIMIT so an agent's communities are never truncated.
    let grouped = community.is_none() && limit == DiscoveryLimit::TargetKeys;
    let mut query = QueryBuilder::<Postgres>::new(if grouped {
        "SELECT array_agg(community_id ORDER BY community_id), encode(pubkey,'hex') FROM users WHERE agent_owner_pubkey=decode("
    } else {
        "SELECT ARRAY[community_id], encode(pubkey,'hex') FROM users WHERE agent_owner_pubkey=decode("
    });
    query
        .push_bind(human.clone())
        .push(", 'hex') AND pubkey<>decode(")
        .push_bind(human.clone())
        .push(", 'hex')");
    if let Some(community) = community {
        query.push(" AND community_id=").push_bind(community);
    }
    if grouped {
        query.push(" GROUP BY pubkey ORDER BY pubkey");
    } else {
        query.push(" ORDER BY community_id, pubkey");
    }
    query.push(" LIMIT ").push_bind((MAX_TARGETS + 1) as i64);
    if lock {
        query.push(" FOR SHARE");
    }
    let rows: Vec<(Vec<Uuid>, String)> = query.build_query_as().fetch_all(conn).await?;
    let count = rows.len() + usize::from(limit == DiscoveryLimit::TargetKeys);
    if count > MAX_TARGETS {
        return Err(DbError::DeletionSafety(
            "user deletion ownership exceeds target limit (10000); no partial scope returned"
                .into(),
        ));
    }
    let mut agents: Vec<_> = rows
        .into_iter()
        .flat_map(|(communities, key)| {
            communities
                .into_iter()
                .map(move |community| (community, key.clone()))
        })
        .collect();
    agents.sort();
    Ok(UserDeletionTargets { human, agents })
}

#[cfg(test)]
#[path = "agent_ownership_postgres_tests.rs"]
mod postgres_tests;
