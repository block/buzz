//! One ownership rule for every user-deletion handler, independent of membership.
use super::normalize_user_deletion_pubkey;
use crate::{DbError, Result};
use sqlx::{PgConnection, Postgres, QueryBuilder};
use uuid::Uuid;

pub(super) const MAX_TARGETS: usize = 10_000;

/// Fresh human and direct-agent identities, retaining each agent's community.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct UserDeletionTargets {
    /// The human remains the global discovery root.
    pub human: String,
    /// Agent authority applies only in the recorded community.
    pub agents: Vec<(Uuid, String)>,
}

impl UserDeletionTargets {
    /// Whether an upload claim belongs to this deletion scope.
    pub fn contains(&self, community: Uuid, pubkey: &str) -> bool {
        pubkey == self.human
            || self
                .agents
                .iter()
                .any(|(c, key)| *c == community && key == pubkey)
    }

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
pub(super) async fn discover(
    conn: &mut PgConnection,
    human: &str,
    community: Option<Uuid>,
    lock: bool,
) -> Result<UserDeletionTargets> {
    let human = normalize_user_deletion_pubkey(human)?;
    let mut query = QueryBuilder::<Postgres>::new(
        "SELECT community_id, encode(pubkey,'hex') FROM users WHERE agent_owner_pubkey=decode(",
    );
    query
        .push_bind(human.clone())
        .push(", 'hex') AND pubkey<>decode(")
        .push_bind(human.clone())
        .push(", 'hex')");
    if let Some(community) = community {
        query.push(" AND community_id=").push_bind(community);
    }
    query
        .push(" ORDER BY community_id, pubkey LIMIT ")
        .push_bind((MAX_TARGETS + 1) as i64);
    if lock {
        query.push(" FOR SHARE");
    }
    let agents = query.build_query_as().fetch_all(conn).await?;
    let targets = UserDeletionTargets { human, agents };
    if targets.agents.len() > MAX_TARGETS || targets.pubkeys().len() > MAX_TARGETS {
        return Err(DbError::DeletionSafety(
            "user deletion ownership exceeds target limit (10000); no partial scope returned"
                .into(),
        ));
    }
    Ok(targets)
}

#[cfg(test)]
#[path = "agent_ownership_postgres_tests.rs"]
mod postgres_tests;
