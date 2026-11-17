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
//! forums take part. A channel filed under a parent may auto-archive (have a
//! TTL); a project, a channel with children and no parent of its own, may
//! not ([`PROJECTS_MAY_AUTO_ARCHIVE`]). Auto-archive only archives, and an
//! archived channel still reads and grants as before. These rules, and the
//! absence of loops, are enforced when a parent or a TTL is written
//! ([`decide_parent_change`], [`refuse_ttl_on_project`]), under row locks on
//! every channel involved.
//!
//! ## The rule: the closest grant wins
//!
//! Walk from the channel up through its parents. At each channel, a person's
//! member row grants its role, and otherwise an open channel grants its
//! `join_role`. The first grant found decides ([`closest_grant`]); no grant
//! anywhere means no access.
//!
//! ## What a channel takes from its parents
//!
//! Each channel's `parent_access` ([`ParentAccess`]) caps what the walk may
//! bring down to it from above:
//!
//! - `member`, the default: roles found above are capped at member, so owner
//!   and admin powers come only from a direct row.
//! - `guest`: roles found above are capped at guest, so the parent's people
//!   read the channel but cannot post in it.
//! - `none`: the walk stops at the channel. Only its own rows and, while it
//!   is open, its own `join_role` grant anything. Its children are cut off
//!   from the channels above it too.
//!
//! Each step up passes the cap of the channel it leaves, so a grandparent's
//! grant reaches a grandchild only through both caps, and the narrower wins.
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

/// What a channel's parents may grant in it: its `parent_access` column. See
/// the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentAccess {
    /// Roles from above cap at member. The default.
    Member,
    /// Roles from above cap at guest: read, never post.
    Guest,
    /// Nothing from above reaches the channel or its children.
    NoAccess,
}

impl ParentAccess {
    /// The stored and published value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Guest => "guest",
            Self::NoAccess => "none",
        }
    }

    /// A stored value. An unknown one fails closed to [`Self::NoAccess`].
    pub fn parse_fail_closed(value: &str) -> Self {
        value.parse().unwrap_or(Self::NoAccess)
    }

    /// The narrower of two caps.
    fn narrower(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoAccess, _) | (_, Self::NoAccess) => Self::NoAccess,
            (Self::Guest, _) | (_, Self::Guest) => Self::Guest,
            (Self::Member, Self::Member) => Self::Member,
        }
    }

    /// `role`, found above a channel with this cap, as it applies there.
    fn cap(self, role: MemberRole) -> Option<MemberRole> {
        match (self, role) {
            (Self::NoAccess, _) => None,
            (Self::Guest, _) => Some(MemberRole::Guest),
            (Self::Member, MemberRole::Owner | MemberRole::Admin) => Some(MemberRole::Member),
            (Self::Member, other) => Some(other),
        }
    }
}

impl std::str::FromStr for ParentAccess {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "member" => Ok(Self::Member),
            "guest" => Ok(Self::Guest),
            "none" => Ok(Self::NoAccess),
            other => Err(format!(
                "invalid parent_access value: {other} (must be \"member\", \"guest\" or \"none\")"
            )),
        }
    }
}

/// The role a person holds in a channel, given the channel and then each of
/// its parents, nearest first: each level's grant, and that level's own
/// [`ParentAccess`]. See the module docs.
pub fn closest_grant(
    levels: impl IntoIterator<Item = (LevelGrant, ParentAccess)>,
) -> Option<MemberRole> {
    // `None` until the walk leaves the channel itself, whose grants are
    // never capped.
    let mut cap: Option<ParentAccess> = None;
    for (grant, parent_access) in levels {
        let role = match grant {
            LevelGrant::Row(role) => Some(role),
            LevelGrant::Open(join_role) => Some(join_role.member_role()),
            LevelGrant::Nothing => None,
        };
        if let Some(role) = role {
            return match cap {
                None => Some(role),
                Some(cap) => cap.cap(role),
            };
        }
        let next = cap.map_or(parent_access, |cap| cap.narrower(parent_access));
        if next == ParentAccess::NoAccess {
            return None;
        }
        cap = Some(next);
    }
    None
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
    /// Its own `parent_access`, which caps what its parents grant.
    pub parent_access: ParentAccess,
}

/// A channel's own `parent_access` and its live parents, nearest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lineage {
    /// The channel's own `parent_access`. `None` for a channel not found.
    pub parent_access: Option<ParentAccess>,
    /// Its live parents, nearest first, at most [`MAX_ANCESTORS`].
    pub ancestors: Vec<Ancestor>,
}

impl Lineage {
    /// The parents whose grants can reach the channel: the walk stops above
    /// any channel whose `parent_access` is `none`.
    pub fn reachable(&self) -> impl Iterator<Item = &Ancestor> {
        let mut gate = self.parent_access.unwrap_or(ParentAccess::NoAccess);
        self.ancestors.iter().map_while(move |ancestor| {
            if gate == ParentAccess::NoAccess {
                return None;
            }
            gate = ancestor.parent_access;
            Some(ancestor)
        })
    }
}

/// `channel_id`'s [`Lineage`]. The walk stops at a deleted parent, which
/// grants nothing.
pub async fn get_channel_ancestors(
    pool: &sqlx::PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Lineage> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    ancestors_on(&mut connection, community_id, channel_id).await
}

/// [`get_channel_ancestors`] on `connection`, for a caller inside a
/// transaction.
async fn ancestors_on(
    connection: &mut sqlx::PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Lineage> {
    let row = sqlx::query(
        "SELECT c.parent_access AS c_parent_access, \
                p.id AS p_id, p.visibility::text AS p_visibility, p.join_role AS p_join_role, \
                p.parent_access AS p_parent_access, \
                g.id AS g_id, g.visibility::text AS g_visibility, g.join_role AS g_join_role, \
                g.parent_access AS g_parent_access \
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
        return Ok(Lineage::default());
    };
    let parent_access = |prefix: &str| -> Result<ParentAccess> {
        let value: String = row.try_get(format!("{prefix}_parent_access").as_str())?;
        Ok(ParentAccess::parse_fail_closed(&value))
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
            parent_access: parent_access(prefix)?,
        });
    }
    Ok(Lineage {
        parent_access: Some(parent_access("c")?),
        ancestors,
    })
}

/// The role `pubkey` holds in `channel_id` through its parents alone, read
/// inside `tx`: the closest grant over the parents, as if the person had no
/// row in the channel itself. `None` means the parents grant nothing.
///
/// The parents' membership rows are read, not locked: a removal from a
/// parent that commits while `tx` is open is not seen. A caller that needs
/// the answer to hold until commit must lock those rows itself.
pub async fn inherited_role_in_transaction(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<MemberRole>> {
    let community_id = tx.community();
    let lineage = ancestors_on(tx.conn(), community_id, channel_id).await?;
    let Some(parent_access) = lineage.parent_access else {
        return Ok(None);
    };
    let mut levels = vec![(LevelGrant::Nothing, parent_access)];
    for ancestor in lineage.reachable() {
        let role = direct_role(tx, ancestor.id, pubkey).await?;
        let level = LevelGrant::of(role.as_deref(), &ancestor.visibility, &ancestor.join_role);
        levels.push((level, ancestor.parent_access));
        if level != LevelGrant::Nothing {
            break;
        }
    }
    Ok(closest_grant(levels))
}

/// SQL true when `$2`, a pubkey, can read channel alias `c` through its
/// parents, given when an open parent grants (`$parent_open` on alias `p`,
/// `$grandparent_open` on alias `g`): `c` has a live parent that `$2` is a
/// member of or that grants, or a live grandparent reached through that
/// parent that is or does. A `parent_access` of `none` stops the walk: on
/// `c`, nothing above grants; on `p`, the grandparent does not. Reading needs
/// any grant, so a `guest` cap does not matter here.
macro_rules! through_parent_sql {
    ($parent_open:literal, $grandparent_open:literal) => {
        concat!(
            "EXISTS ( \
    SELECT 1 FROM channels p \
    LEFT JOIN channels g ON g.community_id = p.community_id AND g.id = p.parent_id \
         AND g.deleted_at IS NULL AND p.parent_access <> 'none' \
    WHERE p.community_id = c.community_id AND p.id = c.parent_id AND p.deleted_at IS NULL \
      AND c.parent_access <> 'none' \
      AND ((",
            $parent_open,
            ") \
           OR EXISTS (SELECT 1 FROM channel_members pm WHERE pm.community_id = p.community_id \
                      AND pm.channel_id = p.id AND pm.pubkey = $2 AND pm.removed_at IS NULL) \
           OR (",
            $grandparent_open,
            ") \
           OR EXISTS (SELECT 1 FROM channel_members gm WHERE gm.community_id = g.community_id \
                      AND gm.channel_id = g.id AND gm.pubkey = $2 AND gm.removed_at IS NULL)))"
        )
    };
}
pub(crate) use through_parent_sql;

/// [`READABLE_THROUGH_PARENT_SQL`] as a literal, for `concat!`.
macro_rules! readable_through_parent_sql {
    () => {
        $crate::channel_tree::through_parent_sql!("p.visibility = 'open'", "g.visibility = 'open'")
    };
}
pub(crate) use readable_through_parent_sql;

/// [`IMPLICITLY_READABLE_THROUGH_PARENT_SQL`] as a literal, for `concat!`.
macro_rules! implicitly_readable_through_parent_sql {
    () => {
        $crate::channel_tree::through_parent_sql!(
            "p.visibility = 'open' AND NOT p.unlisted",
            "g.visibility = 'open' AND NOT g.unlisted"
        )
    };
}
pub(crate) use implicitly_readable_through_parent_sql;

/// SQL true when `$2` can read channel alias `c` through its parents, for a
/// read that names `c`: any open parent grants. Whether `c` itself grants
/// access is the caller's half of the predicate.
pub(crate) const READABLE_THROUGH_PARENT_SQL: &str = readable_through_parent_sql!();

/// [`READABLE_THROUGH_PARENT_SQL`] for a read that does not name `c`: only a
/// listed open parent grants, as in `get_listed_open_channel_ids`. An
/// unlisted parent's children are named-only.
pub(crate) const IMPLICITLY_READABLE_THROUGH_PARENT_SQL: &str =
    implicitly_readable_through_parent_sql!();

/// Push [`READABLE_THROUGH_PARENT_SQL`] onto `query`, binding `pubkey` for
/// each `$2`, for a query built with a [`sqlx::QueryBuilder`].
pub(crate) fn push_readable_through_parent(
    query: &mut sqlx::QueryBuilder<sqlx::Postgres>,
    pubkey: &[u8],
) {
    push_through_parent(query, READABLE_THROUGH_PARENT_SQL, pubkey);
}

/// Push `fragment`, one of the through-parent predicates above, onto
/// `query`, binding `pubkey` for each `$2`.
pub(crate) fn push_through_parent(
    query: &mut sqlx::QueryBuilder<sqlx::Postgres>,
    fragment: &str,
    pubkey: &[u8],
) {
    let mut parts = fragment.split("$2").peekable();
    while let Some(part) = parts.next() {
        query.push(part);
        if parts.peek().is_some() {
            query.push_bind(pubkey.to_vec());
        }
    }
}

/// SQL selecting `id` for every live child and grandchild of the channels in
/// `roots` (a subquery or CTE name yielding a column `id`), whose type passes
/// `types` (a predicate on alias `d`). The parent between a grandchild and its
/// root must be live too.
///
/// With `reached`, only the ones a root's grants reach: no channel on the
/// way down, the descendant included, has `parent_access` `none`. Without
/// it, every descendant, for callers that must find all of them, such as
/// subscription eviction.
pub(crate) fn descendants_sql(roots: &str, types: &str, reached: bool) -> String {
    let (child_gate, grandchild_gate) = if reached {
        (
            " AND d.parent_access <> 'none'",
            " AND d.parent_access <> 'none' AND m.parent_access <> 'none'",
        )
    } else {
        ("", "")
    };
    format!(
        "SELECT d.id FROM channels d JOIN {roots} r ON d.parent_id = r.id \
         WHERE d.community_id = $1 AND d.deleted_at IS NULL AND {types}{child_gate} \
         UNION \
         SELECT d.id FROM channels d \
         JOIN channels m ON m.community_id = d.community_id AND m.id = d.parent_id \
              AND m.deleted_at IS NULL \
         JOIN {roots} r ON m.parent_id = r.id \
         WHERE d.community_id = $1 AND d.deleted_at IS NULL AND {types}{grandchild_gate}"
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
/// Both channels must be streams or forums, and the new parent must be live
/// and unarchived. Neither the new parent nor a child whose parent is
/// cleared may be left an auto-archiving project
/// ([`refuse_auto_archiving_project`]). The tree stays at most [`MAX_ANCESTORS`] + 1
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
        let authorized = elevated_in(actor_role_in_child)
            || match current_parent {
                Some(current) => elevated_in(direct_role(tx, current, actor).await?),
                None => false,
            };
        if !authorized {
            return Err(DbError::AccessDenied(
                "only an owner or admin of the channel or of its parent may clear its parent"
                    .into(),
            ));
        }
        // Without a parent, an auto-archiving channel with children would
        // become an auto-archiving project. A channel filed under `child`
        // locks `child` first, which this transaction holds.
        if child_ttl.is_some() {
            refuse_auto_archiving_project(false, has_live_children(tx, child).await?)?;
        }
        return Ok(());
    };

    if !elevated_in(actor_role_in_child) {
        return Err(DbError::AccessDenied(
            "only an owner or admin of the channel may set its parent".into(),
        ));
    }
    if !PARENTABLE_CHANNEL_TYPES.contains(&child_type.as_str()) {
        return Err(DbError::InvalidData(
            "only a stream or forum can have a parent".into(),
        ));
    }

    // Lock the new parent's chain, nearest first, and collect each level's
    // grant for the actor.
    let mut chain: Vec<Uuid> = Vec::with_capacity(MAX_ANCESTORS + 1);
    let mut grants: Vec<(LevelGrant, ParentAccess)> = Vec::with_capacity(MAX_ANCESTORS + 1);
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
                    AS archived, visibility::text AS visibility, join_role, parent_id, \
                    parent_access \
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
            if !PARENTABLE_CHANNEL_TYPES.contains(&channel_type.as_str()) {
                return Err(DbError::InvalidData(
                    "only a stream or forum can be a parent".into(),
                ));
            }
            if ttl.is_some() {
                let has_parent = row.try_get::<Option<Uuid>, _>("parent_id")?.is_some();
                refuse_auto_archiving_project(has_parent, true)?;
            }
            if archived {
                return Err(DbError::AccessDenied(
                    "the parent channel is archived".into(),
                ));
            }
        }
        let visibility: String = row.try_get("visibility")?;
        let join_role: String = row.try_get("join_role")?;
        let parent_access: String = row.try_get("parent_access")?;
        let role = direct_role(tx, id, actor).await?;
        grants.push((
            LevelGrant::of(role.as_deref(), &visibility, &join_role),
            ParentAccess::parse_fail_closed(&parent_access),
        ));
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

/// Why a channel cannot hold an artifact's conversation.
pub const CONVERSATION_HOME_REFUSED: &str =
    "comments aren't available here: only a stream or forum at most two levels deep can hold them";

/// Lock, on `tx`, the channel that holds or will hold an artifact's
/// conversation, and its parents nearest first (`FOR NO KEY UPDATE`, as
/// [`decide_parent_change`] locks a new parent's chain), and say why it
/// cannot hold one. A conversation is filed under its artifact's home, so the
/// home must be a live, unarchived stream or forum with at most one parent:
/// the conversation is then at most the third level. Filing the home or one
/// of its parents elsewhere counts the conversation in the subtree height,
/// and waits on these locks, so the depth cannot change before commit.
pub(crate) async fn refuse_conversation_home(
    tx: &mut crate::AdmittedTx,
    home: Uuid,
) -> Result<Option<&'static str>> {
    let community_id = tx.community();
    let mut levels = 0;
    let mut next = Some(home);
    while let Some(id) = next {
        if levels == MAX_ANCESTORS {
            return Ok(Some(CONVERSATION_HOME_REFUSED));
        }
        let row = sqlx::query(
            "SELECT channel_type::text AS channel_type, archived_at IS NOT NULL AS archived, \
                    deleted_at IS NOT NULL AS deleted, parent_id \
             FROM channels WHERE community_id = $1 AND id = $2 FOR NO KEY UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(id)
        .fetch_optional(tx.conn())
        .await?;
        let Some(row) = row else {
            return Ok(Some(CONVERSATION_HOME_REFUSED));
        };
        if id == home {
            let channel_type: String = row.try_get("channel_type")?;
            if !PARENTABLE_CHANNEL_TYPES.contains(&channel_type.as_str())
                || row.try_get::<bool, _>("deleted")?
            {
                return Ok(Some(CONVERSATION_HOME_REFUSED));
            }
            if row.try_get::<bool, _>("archived")? {
                return Ok(Some("channel is archived"));
            }
        }
        levels += 1;
        next = row.try_get("parent_id")?;
    }
    Ok(None)
}

/// Decide, on `tx`, whether `actor` may change `channel_id`'s
/// `parent_access`: only a direct owner or admin row may. The caller holds
/// the channel's row lock. Either direction changes who reads the channel
/// and its children, so neither is open to a parent's admin.
pub(crate) async fn decide_parent_access_change(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    actor: &[u8],
) -> Result<()> {
    let elevated = direct_role(tx, channel_id, actor)
        .await?
        .and_then(|role| role.parse::<MemberRole>().ok())
        .is_some_and(|role| role.is_elevated());
    if elevated {
        Ok(())
    } else {
        Err(DbError::AccessDenied(
            "only an owner or admin of the channel may change what its parents grant".into(),
        ))
    }
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

/// Whether a project, a channel with live children and no parent of its
/// own, may auto-archive (have a TTL). A channel filed under a parent may,
/// whatever it holds. Allowing projects too means flipping this, the one
/// check every path goes through ([`refuse_auto_archiving_project`]).
const PROJECTS_MAY_AUTO_ARCHIVE: bool = false;

/// Refuse leaving an auto-archiving channel as a project: with live children
/// and no parent.
fn refuse_auto_archiving_project(has_parent: bool, has_live_children: bool) -> Result<()> {
    if PROJECTS_MAY_AUTO_ARCHIVE || has_parent || !has_live_children {
        return Ok(());
    }
    Err(DbError::InvalidData(
        "a project (a channel with children and no parent) cannot have a ttl".into(),
    ))
}

/// Whether `channel_id` has a live child. An artifact's conversation is not
/// one: holding a task's comments does not make a channel a project.
async fn has_live_children(tx: &mut crate::AdmittedTx, channel_id: Uuid) -> Result<bool> {
    let community_id = tx.community();
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM channels WHERE community_id = $1 AND parent_id = $2 \
                        AND deleted_at IS NULL AND channel_type <> 'artifact')",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?)
}

/// Refuse, on `tx`, giving `channel_id` a TTL while it is a project
/// ([`refuse_auto_archiving_project`]). The caller holds `channel_id`'s row
/// lock, and filing a channel under it locks it too, so neither can slip
/// past the other.
pub(crate) async fn refuse_ttl_on_project(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
) -> Result<()> {
    let community_id = tx.community();
    let has_parent: bool = sqlx::query_scalar(
        "SELECT parent_id IS NOT NULL FROM channels WHERE community_id = $1 AND id = $2",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_one(tx.conn())
    .await?;
    refuse_auto_archiving_project(has_parent, has_live_children(tx, channel_id).await?)
}

/// The live private channels under `channel_id`, children and grandchildren.
/// Their readers through `channel_id` change when its access does.
pub async fn get_private_descendant_ids(
    pool: &sqlx::PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<Vec<Uuid>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let sql = format!(
        "WITH roots AS (SELECT $2::uuid AS id), d AS ({}) \
         SELECT c.id FROM channels c JOIN d ON d.id = c.id \
         WHERE c.community_id = $1 AND c.visibility = 'private'",
        descendants_sql("roots", "TRUE", false)
    );
    // Every part of `sql` is a literal; the ids are bound.
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_all(&mut *connection)
        .await?)
}

impl crate::Db {
    /// See [`get_private_descendant_ids`].
    pub async fn get_private_descendant_ids(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        get_private_descendant_ids(&self.pool, community_id, channel_id).await
    }

    /// See [`get_channel_ancestors`].
    pub async fn get_channel_ancestors(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) -> Result<Lineage> {
        get_channel_ancestors(&self.pool, community_id, channel_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LevelGrant::{Nothing, Open, Row};
    use MemberRole::{Admin, Bot, Guest, Member, Owner};

    const ALL_ROLES: [MemberRole; 5] = [Owner, Admin, Member, Guest, Bot];

    /// [`closest_grant`] over levels that all take what their parents grant
    /// up to member, the default.
    fn walk(levels: &[LevelGrant]) -> Option<MemberRole> {
        closest_grant(levels.iter().map(|level| (*level, ParentAccess::Member)))
    }

    #[test]
    fn a_channel_without_parents_decides_as_before() {
        for role in ALL_ROLES {
            assert_eq!(walk(&[Row(role)]), Some(role), "direct {role:?}");
        }
        assert_eq!(walk(&[Open(ChannelJoinRole::Member)]), Some(Member));
        assert_eq!(walk(&[Open(ChannelJoinRole::Guest)]), Some(Guest));
        assert_eq!(walk(&[Nothing]), None);
        assert_eq!(walk(&[]), None);
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
                walk(&[Nothing, Row(role)]),
                Some(expected),
                "parent {role:?}"
            );
            assert_eq!(
                walk(&[Nothing, Nothing, Row(role)]),
                Some(expected),
                "grandparent {role:?}"
            );
        }
    }

    #[test]
    fn the_closest_grant_wins() {
        // A direct row beats everything above, guest included.
        assert_eq!(walk(&[Row(Guest), Row(Owner)]), Some(Guest));
        assert_eq!(
            walk(&[Row(Guest), Open(ChannelJoinRole::Member)]),
            Some(Guest)
        );
        // An open channel's join role is the grant at that level, so an
        // announce channel stays read-only for a project member.
        assert_eq!(
            walk(&[Open(ChannelJoinRole::Guest), Row(Member)]),
            Some(Guest)
        );
        assert_eq!(
            walk(&[Open(ChannelJoinRole::Member), Row(Guest)]),
            Some(Member)
        );
        // A guest row on the parent stops the walk before the grandparent.
        assert_eq!(walk(&[Nothing, Row(Guest), Row(Member)]), Some(Guest));
        // An open announce parent makes a child's readers guests.
        assert_eq!(walk(&[Nothing, Open(ChannelJoinRole::Guest)]), Some(Guest));
        assert_eq!(
            walk(&[Nothing, Open(ChannelJoinRole::Member), Row(Guest)]),
            Some(Member)
        );
        // Private all the way up grants nothing.
        assert_eq!(walk(&[Nothing, Nothing, Nothing]), None);
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

    #[test]
    fn a_guest_cap_makes_inherited_readers_guests() {
        use ParentAccess::Guest as ViewOnly;
        // Whatever the parent grants, the child gives guest.
        for role in ALL_ROLES {
            assert_eq!(
                closest_grant([(Nothing, ViewOnly), (Row(role), ParentAccess::Member)]),
                Some(Guest),
                "parent {role:?}"
            );
        }
        assert_eq!(
            closest_grant([
                (Nothing, ViewOnly),
                (Open(ChannelJoinRole::Member), ParentAccess::Member)
            ]),
            Some(Guest)
        );
        // A direct row still decides, in either direction.
        assert_eq!(
            closest_grant([(Row(Member), ViewOnly), (Row(Admin), ParentAccess::Member)]),
            Some(Member)
        );
        assert_eq!(closest_grant([(Row(Admin), ViewOnly)]), Some(Admin));
        // The cap carries up: a grandparent's grant passes the child's cap.
        assert_eq!(
            closest_grant([
                (Nothing, ViewOnly),
                (Nothing, ParentAccess::Member),
                (Row(Owner), ParentAccess::Member)
            ]),
            Some(Guest)
        );
        // And a middle channel's cap binds its children.
        assert_eq!(
            closest_grant([
                (Nothing, ParentAccess::Member),
                (Nothing, ViewOnly),
                (Row(Member), ParentAccess::Member)
            ]),
            Some(Guest)
        );
    }

    #[test]
    fn no_access_stops_the_walk() {
        use ParentAccess::NoAccess;
        for role in ALL_ROLES {
            assert_eq!(
                closest_grant([(Nothing, NoAccess), (Row(role), ParentAccess::Member)]),
                None,
                "parent {role:?}"
            );
            // A restricted middle channel cuts its children off from the
            // grandparent, but not from itself.
            assert_eq!(
                closest_grant([
                    (Nothing, ParentAccess::Member),
                    (Nothing, NoAccess),
                    (Row(role), ParentAccess::Member)
                ]),
                None,
                "grandparent {role:?}"
            );
        }
        assert_eq!(
            closest_grant([
                (Nothing, ParentAccess::Member),
                (Row(Member), NoAccess),
                (Row(Owner), ParentAccess::Member)
            ]),
            Some(Member)
        );
        assert_eq!(
            closest_grant([
                (Nothing, NoAccess),
                (Open(ChannelJoinRole::Member), ParentAccess::Member)
            ]),
            None
        );
        // The channel's own grants are untouched.
        assert_eq!(closest_grant([(Row(Guest), NoAccess)]), Some(Guest));
        assert_eq!(
            closest_grant([(Open(ChannelJoinRole::Member), NoAccess)]),
            Some(Member)
        );
    }

    #[test]
    fn lineage_reaches_parents_up_to_a_none() {
        let ancestor = |n: u128, parent_access| Ancestor {
            id: Uuid::from_u128(n),
            visibility: "private".into(),
            join_role: "member".into(),
            parent_access,
        };
        let ids = |lineage: &Lineage| {
            lineage
                .reachable()
                .map(|a| a.id.as_u128())
                .collect::<Vec<_>>()
        };
        let two = |child, middle| Lineage {
            parent_access: Some(child),
            ancestors: vec![ancestor(1, middle), ancestor(2, ParentAccess::Member)],
        };
        assert_eq!(
            ids(&two(ParentAccess::Member, ParentAccess::Member)),
            [1, 2]
        );
        assert_eq!(ids(&two(ParentAccess::Guest, ParentAccess::Guest)), [1, 2]);
        assert_eq!(ids(&two(ParentAccess::Member, ParentAccess::NoAccess)), [1]);
        assert!(ids(&two(ParentAccess::NoAccess, ParentAccess::Member)).is_empty());
        assert!(ids(&Lineage::default()).is_empty());
    }

    #[test]
    fn parent_access_values_round_trip_and_fail_closed() {
        for access in [
            ParentAccess::Member,
            ParentAccess::Guest,
            ParentAccess::NoAccess,
        ] {
            assert_eq!(access.as_str().parse::<ParentAccess>(), Ok(access));
            assert_eq!(ParentAccess::parse_fail_closed(access.as_str()), access);
        }
        assert!("bogus".parse::<ParentAccess>().is_err());
        assert!("".parse::<ParentAccess>().is_err());
        assert_eq!(
            ParentAccess::parse_fail_closed("bogus"),
            ParentAccess::NoAccess
        );
    }
}
