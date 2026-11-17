//! Reading and writing a private channel through its parents.
//!
//! The rule lives in [`buzz_db::channel_tree`]: the closest grant wins, and a
//! parent matters only for a private channel where the person has no row. An
//! open channel or a direct row is decided by the caller exactly as before,
//! so these helpers are reached only on that one path. Parent membership is
//! read through the per-person membership cache, which a membership change
//! already invalidates, so no answer here is cached per channel.

use std::collections::HashSet;

use buzz_core::channel::MemberRole;
use buzz_core::CommunityId;
use buzz_db::channel::ChannelRecord;
use buzz_db::channel_tree::{closest_grant, LevelGrant};

use crate::state::AppState;

/// The role `pubkey` holds in private `channel` through its parents, when it
/// has no row there. `None` means no access.
pub(crate) async fn inherited_role(
    state: &AppState,
    community_id: CommunityId,
    channel: &ChannelRecord,
    pubkey: &[u8],
) -> Result<Option<MemberRole>, buzz_db::DbError> {
    if channel.parent_id.is_none() || channel.visibility == "open" {
        return Ok(None);
    }
    let lineage = state
        .db
        .get_channel_ancestors(community_id, channel.id)
        .await?;
    let Some(parent_access) = lineage.parent_access else {
        return Ok(None);
    };
    let mut levels = vec![(LevelGrant::Nothing, parent_access)];
    for ancestor in lineage.reachable() {
        let role = if state
            .is_member_cached(community_id, ancestor.id, pubkey)
            .await?
        {
            state
                .db
                .get_member_role(community_id, ancestor.id, pubkey)
                .await?
        } else {
            None
        };
        let level = LevelGrant::of(role.as_deref(), &ancestor.visibility, &ancestor.join_role);
        levels.push((level, ancestor.parent_access));
        if level != LevelGrant::Nothing {
            break;
        }
    }
    Ok(closest_grant(levels))
}

/// Which of `pubkeys`, none of them members of private channel `channel_id`,
/// can read it through its parents. For a fan-out's recipients: one bulk
/// membership read per parent instead of one read each. Reading needs any
/// grant, so a guest of a parent reads the child too, and a `guest` cap does
/// not matter; a `none` one stops the walk ([`Lineage::reachable`]).
///
/// [`Lineage::reachable`]: buzz_db::channel_tree::Lineage::reachable
pub(crate) async fn inherited_readers(
    state: &AppState,
    community_id: CommunityId,
    channel_id: uuid::Uuid,
    pubkeys: Vec<Vec<u8>>,
) -> crate::membership_cache::Members {
    let mut answer = crate::membership_cache::Members::default();
    if pubkeys.is_empty() {
        return answer;
    }
    let lineage = match state
        .db
        .get_channel_ancestors(community_id, channel_id)
        .await
    {
        Ok(lineage) => lineage,
        Err(error) => {
            tracing::warn!(%channel_id, "fan-out parent access: ancestor read failed: {error}");
            answer.unverified.extend(pubkeys);
            return answer;
        }
    };
    let mut remaining: HashSet<Vec<u8>> = pubkeys.into_iter().collect();
    for ancestor in lineage.reachable() {
        if remaining.is_empty() {
            break;
        }
        if ancestor.visibility == "open" {
            answer.members.extend(remaining.drain());
            break;
        }
        let found = state
            .members_cached(community_id, ancestor.id, remaining.iter().cloned())
            .await;
        for pubkey in found.members.iter().chain(found.unverified.iter()) {
            remaining.remove(pubkey);
        }
        answer.members.extend(found.members);
        answer.unverified.extend(found.unverified);
    }
    answer
}
