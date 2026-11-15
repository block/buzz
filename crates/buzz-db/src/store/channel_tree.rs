//! Channel parents: reading a channel through the channel it is filed under.
//!
//! A channel may name one parent (`channels.parent_id`). Anyone who can read
//! the parent can read the child, so a project is just a channel that other
//! channels name as their parent. Access through a parent never creates a
//! membership row: membership still means "joined".
//!
//! ## Shape
//!
//! A tree is at most three channels deep: a top channel, a child, and a
//! grandchild ([`MAX_ANCESTORS`] parents above any channel). Only streams and
//! forums take part, never an ephemeral (TTL) channel. Both rules, and the
//! absence of loops, are enforced when a parent is written
//! ([`decide_parent_change`]), under row locks on every channel involved.
//!
//! ## The rule: the closest grant wins
//!
//! Walk from the channel up through its parents. At each channel, a person's
//! member row grants its role, and otherwise an open channel grants its
//! `join_role`. The first grant found decides ([`closest_grant`]); no grant
//! anywhere means no access. Roles found above the channel itself are capped
//! at member, so owner and admin powers come only from a direct row.
//!
//! Because the channel itself is the first level, an open channel or a direct
//! row decides exactly as it did before parents existed. A parent matters only
//! for a private channel where the person has no row.

use buzz_core::channel::{ChannelJoinRole, MemberRole};
use buzz_core::CommunityId;
use sqlx::Row;
use uuid::Uuid;

use crate::error::{DbError, Result};

/// The most parents above any channel: a tree is at most three deep.
pub const MAX_ANCESTORS: usize = 2;

/// Channel types that may be a parent or a child.
pub const PARENTABLE_CHANNEL_TYPES: &[&str] = &["stream", "forum"];

/// What one channel on a chain grants a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelGrant {
    /// The person has an active member row with this role.
    Row(MemberRole),
    /// The person has no row, and the channel is open with this `join_role`.
    Open(ChannelJoinRole),
    /// The person has no row, and the channel is private.
    Nothing,
}

impl LevelGrant {
    /// The grant a channel with `visibility` and `join_role` gives a person
    /// whose stored role there is `role`. An unknown stored role grants
    /// nothing, and an unknown `join_role` fails closed to guest.
    pub fn of(role: Option<&str>, visibility: &str, join_role: &str) -> Self {
        match role {
            Some(role) => role.parse().map_or(Self::Nothing, Self::Row),
            None if visibility == "open" => {
                Self::Open(ChannelJoinRole::parse_fail_closed(join_role))
            }
            None => Self::Nothing,
        }
    }
}

/// The role a person holds in a channel, given the grants of the channel and
/// then each of its parents, nearest first. See the module docs.
pub fn closest_grant(levels: impl IntoIterator<Item = LevelGrant>) -> Option<MemberRole> {
    for (depth, level) in levels.into_iter().enumerate() {
        match level {
            LevelGrant::Row(role) if depth == 0 => return Some(role),
            LevelGrant::Row(role) => return Some(inherited(role)),
            LevelGrant::Open(join_role) => return Some(join_role.member_role()),
            LevelGrant::Nothing => {}
        }
    }
    None
}

/// A role found on a parent, as it applies to the child.
fn inherited(role: MemberRole) -> MemberRole {
    match role {
        MemberRole::Owner | MemberRole::Admin => MemberRole::Member,
        other => other,
    }
}

/// One parent of a channel, nearest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ancestor {
    /// The parent channel.
    pub id: Uuid,
    /// Its visibility (`"open"` or `"private"`).
    pub visibility: String,
    /// Its `join_role`.
    pub join_role: String,
}

/// The live parents of `channel_id`, nearest first, at most
/// [`MAX_ANCESTORS`]. The walk stops at a deleted parent, which grants
/// nothing.
pub async fn get_channel_ancestors(
    pool: &sqlx::PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Vec<Ancestor>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        "SELECT p.id AS p_id, p.visibility::text AS p_visibility, p.join_role AS p_join_role, \
                g.id AS g_id, g.visibility::text AS g_visibility, g.join_role AS g_join_role \
         FROM channels c \
         LEFT JOIN channels p ON p.community_id = c.community_id AND p.id = c.parent_id \
              AND p.deleted_at IS NULL \
         LEFT JOIN channels g ON g.community_id = p.community_id AND g.id = p.parent_id \
              AND g.deleted_at IS NULL \
         WHERE c.community_id = $1 AND c.id = $2 AND c.deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let mut ancestors = Vec::with_capacity(MAX_ANCESTORS);
    for prefix in ["p", "g"] {
        let Some(id) = row.try_get::<Option<Uuid>, _>(format!("{prefix}_id").as_str())? else {
            break;
        };
        ancestors.push(Ancestor {
            id,
            visibility: row.try_get(format!("{prefix}_visibility").as_str())?,
            join_role: row.try_get(format!("{prefix}_join_role").as_str())?,
        });
    }
    Ok(ancestors)
}

/// SQL true when `$2`, a pubkey, can read channel alias `c` through its
/// parents: `c` has a live parent that `$2` is a member of or that is open,
/// or a live grandparent reached through that parent that is. Whether `c`
/// itself grants access is the caller's half of the predicate.
pub(crate) const READABLE_THROUGH_PARENT_SQL: &str = "EXISTS ( \
    SELECT 1 FROM channels p \
    LEFT JOIN channels g ON g.community_id = p.community_id AND g.id = p.parent_id \
         AND g.deleted_at IS NULL \
    WHERE p.community_id = c.community_id AND p.id = c.parent_id AND p.deleted_at IS NULL \
      AND (p.visibility = 'open' \
           OR EXISTS (SELECT 1 FROM channel_members pm WHERE pm.community_id = p.community_id \
                      AND pm.channel_id = p.id AND pm.pubkey = $2 AND pm.removed_at IS NULL) \
           OR g.visibility = 'open' \
           OR EXISTS (SELECT 1 FROM channel_members gm WHERE gm.community_id = g.community_id \
                      AND gm.channel_id = g.id AND gm.pubkey = $2 AND gm.removed_at IS NULL)))";

/// SQL selecting `id` for every live child and grandchild of the channels in
/// `roots` (a subquery or CTE name yielding a column `id`), whose type passes
/// `types` (a predicate on alias `d`). The parent between a grandchild and its
/// root must be live too.
pub(crate) fn descendants_sql(roots: &str, types: &str) -> String {
    format!(
        "SELECT d.id FROM channels d JOIN {roots} r ON d.parent_id = r.id \
         WHERE d.community_id = $1 AND d.deleted_at IS NULL AND {types} \
         UNION \
         SELECT d.id FROM channels d \
         JOIN channels m ON m.community_id = d.community_id AND m.id = d.parent_id \
              AND m.deleted_at IS NULL \
         JOIN {roots} r ON m.parent_id = r.id \
         WHERE d.community_id = $1 AND d.deleted_at IS NULL AND {types}"
    )
}

/// A requested change to a channel's parent: `Some` sets it, `None` clears it.
pub type ParentChange = Option<Uuid>;

/// Decide, on `tx`, whether `actor` may give `child` the parent `change`, and
/// refuse it otherwise. The caller holds `child`'s row lock, so a concurrent
/// change under `child` waits for this transaction.
///
/// - **Setting** needs a direct owner or admin row in the child, and a role
///   that may post in the new parent ([`closest_grant`] over its own chain).
///   The new parent and its parents are locked `FOR NO KEY UPDATE`, nearest
///   first, so a concurrent change to any of them waits: a depth or loop
///   check here cannot be invalidated before commit. Two changes that lock
///   the same channels in opposite orders deadlock, and Postgres aborts one,
///   which fails closed.
/// - **Clearing** needs a direct owner or admin row in the child or in its
///   current parent. Clearing only narrows who can read the child.
///
/// Both channels must be streams or forums with no TTL, and the new parent
/// must be live and unarchived. The tree stays at most [`MAX_ANCESTORS`] + 1
/// deep and loop-free.
pub(crate) async fn decide_parent_change(
    tx: &mut crate::AdmittedTx,
    child: Uuid,
    change: ParentChange,
    actor: &[u8],
) -> Result<()> {
    let community_id = tx.community();
    let (child_type, child_ttl, current_parent): (String, Option<i32>, Option<Uuid>) =
        sqlx::query_as(
            "SELECT channel_type::text, ttl_seconds, parent_id FROM channels \
             WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
        )
        .bind(community_id.as_uuid())
        .bind(child)
        .fetch_optional(tx.conn())
        .await?
        .ok_or(DbError::ChannelNotFound(child))?;
    let elevated_in = |role: Option<String>| {
        role.and_then(|role| role.parse::<MemberRole>().ok())
            .is_some_and(|role| role.is_elevated())
    };
    let actor_role_in_child = direct_role(tx, child, actor).await?;

    let Some(parent) = change else {
        if elevated_in(actor_role_in_child) {
            return Ok(());
        }
        if let Some(current) = current_parent {
            if elevated_in(direct_role(tx, current, actor).await?) {
                return Ok(());
            }
        }
        return Err(DbError::AccessDenied(
            "only an owner or admin of the channel or of its parent may clear its parent".into(),
        ));
    };

    if !elevated_in(actor_role_in_child) {
        return Err(DbError::AccessDenied(
            "only an owner or admin of the channel may set its parent".into(),
        ));
    }
    if !PARENTABLE_CHANNEL_TYPES.contains(&child_type.as_str()) || child_ttl.is_some() {
        return Err(DbError::InvalidData(
            "only a stream or forum without a ttl can have a parent".into(),
        ));
    }

    // Lock the new parent's chain, nearest first, and collect each level's
    // grant for the actor.
    let mut chain: Vec<Uuid> = Vec::with_capacity(MAX_ANCESTORS + 1);
    let mut grants: Vec<LevelGrant> = Vec::with_capacity(MAX_ANCESTORS + 1);
    let mut next = Some(parent);
    while let Some(id) = next {
        if id == child || chain.contains(&id) {
            return Err(DbError::InvalidData(
                "a channel cannot be filed under itself or one of its children".into(),
            ));
        }
        if chain.len() == MAX_ANCESTORS {
            return Err(DbError::InvalidData(format!(
                "channels nest at most {} deep",
                MAX_ANCESTORS + 1
            )));
        }
        let row = sqlx::query(
            "SELECT channel_type::text AS channel_type, ttl_seconds, archived_at IS NOT NULL \
                    AS archived, visibility::text AS visibility, join_role, parent_id \
             FROM channels WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL \
             FOR NO KEY UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(id)
        .fetch_optional(tx.conn())
        .await?
        .ok_or(DbError::ChannelNotFound(id))?;
        if id == parent {
            let channel_type: String = row.try_get("channel_type")?;
            let ttl: Option<i32> = row.try_get("ttl_seconds")?;
            let archived: bool = row.try_get("archived")?;
            if !PARENTABLE_CHANNEL_TYPES.contains(&channel_type.as_str()) || ttl.is_some() {
                return Err(DbError::InvalidData(
                    "only a stream or forum without a ttl can be a parent".into(),
                ));
            }
            if archived {
                return Err(DbError::AccessDenied(
                    "the parent channel is archived".into(),
                ));
            }
        }
        let visibility: String = row.try_get("visibility")?;
        let join_role: String = row.try_get("join_role")?;
        let role = direct_role(tx, id, actor).await?;
        grants.push(LevelGrant::of(role.as_deref(), &visibility, &join_role));
        chain.push(id);
        next = row.try_get("parent_id")?;
    }

    // The child's own subtree must fit under the new parent's chain. A
    // channel filed under `child` locks `child` first, which this
    // transaction holds, so the subtree cannot grow before commit.
    let height = subtree_height(tx, child).await?;
    if chain.len() + height > MAX_ANCESTORS {
        return Err(DbError::InvalidData(format!(
            "channels nest at most {} deep",
            MAX_ANCESTORS + 1
        )));
    }

    if !closest_grant(grants).is_some_and(|role| role.can_post()) {
        return Err(DbError::AccessDenied(
            "only someone who may post in the parent may file a channel under it".into(),
        ));
    }
    Ok(())
}

/// The actor's active, direct role in `channel_id`, as stored.
async fn direct_role(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<String>> {
    let community_id = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT role::text FROM channel_members \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 AND removed_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(pubkey)
    .fetch_optional(tx.conn())
    .await?)
}

/// How many levels of channels sit under `channel_id`: 0, 1 or 2. Deleted
/// channels count, so the cap holds whatever happens to them later.
async fn subtree_height(tx: &mut crate::AdmittedTx, channel_id: Uuid) -> Result<usize> {
    let community_id = tx.community();
    let (children, grandchildren): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM channels WHERE community_id = $1 AND parent_id = $2), \
                EXISTS (SELECT 1 FROM channels d JOIN channels m \
                        ON m.community_id = d.community_id AND m.id = d.parent_id \
                        WHERE m.community_id = $1 AND m.parent_id = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?;
    Ok(match (children, grandchildren) {
        (_, true) => 2,
        (true, false) => 1,
        (false, false) => 0,
    })
}

impl crate::Db {
    /// See [`get_channel_ancestors`].
    pub async fn get_channel_ancestors(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<Ancestor>> {
        get_channel_ancestors(&self.pool, community_id, channel_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LevelGrant::{Nothing, Open, Row};
    use MemberRole::{Admin, Bot, Guest, Member, Owner};

    const ALL_ROLES: [MemberRole; 5] = [Owner, Admin, Member, Guest, Bot];

    #[test]
    fn a_channel_without_parents_decides_as_before() {
        for role in ALL_ROLES {
            assert_eq!(closest_grant([Row(role)]), Some(role), "direct {role:?}");
        }
        assert_eq!(closest_grant([Open(ChannelJoinRole::Member)]), Some(Member));
        assert_eq!(closest_grant([Open(ChannelJoinRole::Guest)]), Some(Guest));
        assert_eq!(closest_grant([Nothing]), None);
        assert_eq!(closest_grant([]), None);
    }

    #[test]
    fn inherited_roles_cap_at_member() {
        for (role, expected) in [
            (Owner, Member),
            (Admin, Member),
            (Member, Member),
            (Guest, Guest),
            (Bot, Bot),
        ] {
            assert_eq!(
                closest_grant([Nothing, Row(role)]),
                Some(expected),
                "parent {role:?}"
            );
            assert_eq!(
                closest_grant([Nothing, Nothing, Row(role)]),
                Some(expected),
                "grandparent {role:?}"
            );
        }
    }

    #[test]
    fn the_closest_grant_wins() {
        // A direct row beats everything above, guest included.
        assert_eq!(closest_grant([Row(Guest), Row(Owner)]), Some(Guest));
        assert_eq!(
            closest_grant([Row(Guest), Open(ChannelJoinRole::Member)]),
            Some(Guest)
        );
        // An open channel's join role is the grant at that level, so an
        // announce channel stays read-only for a project member.
        assert_eq!(
            closest_grant([Open(ChannelJoinRole::Guest), Row(Member)]),
            Some(Guest)
        );
        assert_eq!(
            closest_grant([Open(ChannelJoinRole::Member), Row(Guest)]),
            Some(Member)
        );
        // A guest row on the parent stops the walk before the grandparent.
        assert_eq!(
            closest_grant([Nothing, Row(Guest), Row(Member)]),
            Some(Guest)
        );
        // An open announce parent makes a child's readers guests.
        assert_eq!(
            closest_grant([Nothing, Open(ChannelJoinRole::Guest)]),
            Some(Guest)
        );
        assert_eq!(
            closest_grant([Nothing, Open(ChannelJoinRole::Member), Row(Guest)]),
            Some(Member)
        );
        // Private all the way up grants nothing.
        assert_eq!(closest_grant([Nothing, Nothing, Nothing]), None);
    }

    #[test]
    fn level_grant_reads_stored_values_fail_closed() {
        assert_eq!(
            LevelGrant::of(Some("admin"), "private", "member"),
            Row(Admin)
        );
        assert_eq!(LevelGrant::of(Some("bogus"), "open", "member"), Nothing);
        assert_eq!(
            LevelGrant::of(None, "open", "member"),
            Open(ChannelJoinRole::Member)
        );
        assert_eq!(
            LevelGrant::of(None, "open", "bogus"),
            Open(ChannelJoinRole::Guest)
        );
        assert_eq!(LevelGrant::of(None, "private", "member"), Nothing);
    }
}
