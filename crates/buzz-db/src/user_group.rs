//! Community-scoped user-group persistence.
//!
//! User groups are soft-deleted shared objects. Their handles are unique among
//! active groups within a community, and membership/default-channel writes are
//! transactionally fenced against concurrent group deletion.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder, Row as _, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};
use buzz_core::CommunityId;

const ACTIVE_HANDLE_INDEX: &str = "idx_user_groups_active_handle";

/// A user-group metadata row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserGroupRecord {
    /// Stable group UUID within the community.
    pub id: Uuid,
    /// Unique active mention handle.
    pub handle: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
    /// Hex pubkey of the group creator.
    pub created_by: String,
    /// When the group was created.
    pub created_at: DateTime<Utc>,
    /// When metadata, membership, or default channels were last updated.
    pub updated_at: DateTime<Utc>,
    /// When the group was soft-deleted, if applicable.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A user-group membership row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserGroupMemberRecord {
    /// Group containing the member.
    pub group_id: Uuid,
    /// 64-character hex member pubkey.
    pub pubkey: String,
    /// Hex pubkey of the actor who added the member.
    pub added_by: String,
    /// When the member was added.
    pub added_at: DateTime<Utc>,
}

/// Result of adding members to a user group.
///
/// The default channels are read while holding the same group-row lock used
/// for insertion. A relay can therefore auto-join only `added_pubkeys` to this
/// exact channel snapshot without making later default-list edits retroactive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddMembersResult {
    /// Pubkeys newly inserted by this operation, in lexical order.
    pub added_pubkeys: Vec<String>,
    /// Default channels observed atomically with the membership insert.
    pub default_channels: Vec<Uuid>,
}

/// Partial user-group update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserGroupUpdate {
    /// New mention handle, or `None` to leave it unchanged.
    pub handle: Option<String>,
    /// New display name, or `None` to leave it unchanged.
    pub name: Option<String>,
    /// Description change: outer `None` leaves it unchanged, `Some(None)`
    /// clears it, and `Some(Some(value))` replaces it.
    pub description: Option<Option<String>>,
    /// Full replacement default-channel list, or `None` to leave it unchanged.
    /// An empty vector clears the list.
    pub default_channels: Option<Vec<Uuid>>,
}

/// Creates a user group, including its initial members and default channels.
///
/// The insert is atomic across all three user-group tables. A handle already
/// used by an active group returns [`DbError::UserGroupHandleConflict`].
#[allow(clippy::too_many_arguments)]
pub async fn create_group(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
    handle: &str,
    name: &str,
    description: Option<&str>,
    created_by: &str,
    members: &[String],
    default_channels: &[Uuid],
) -> Result<UserGroupRecord> {
    if group_id.is_nil() {
        return Err(DbError::InvalidData(
            "user group id must not be nil".to_owned(),
        ));
    }
    if !buzz_core::user_group::is_valid_group_handle(handle) {
        return Err(DbError::InvalidData(
            "user group handle must match ^[a-z0-9][a-z0-9_-]{1,31}$".to_owned(),
        ));
    }

    let mut tx = pool.begin().await?;
    let row = match sqlx::query(
        "INSERT INTO user_groups \
         (community_id, id, handle, name, description, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         RETURNING id, handle, name, description, created_by, created_at, updated_at, deleted_at",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .bind(handle)
    .bind(name)
    .bind(description)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(error) => return Err(map_handle_conflict(error, handle)),
    };

    let record = row_to_group_record(row)?;
    let _ = insert_members_tx(&mut tx, community, group_id, members, created_by).await?;
    insert_default_channels_tx(&mut tx, community, group_id, default_channels).await?;
    tx.commit().await?;
    Ok(record)
}

/// Returns an active user group by community-scoped UUID.
pub async fn get_group_by_id(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
) -> Result<UserGroupRecord> {
    let row = sqlx::query(
        "SELECT id, handle, name, description, created_by, created_at, updated_at, deleted_at \
         FROM user_groups \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_optional(pool)
    .await?
    .ok_or(DbError::UserGroupNotFound(group_id))?;

    row_to_group_record(row)
}

/// Returns an active user group by its community-scoped mention handle.
pub async fn get_group_by_handle(
    pool: &PgPool,
    community: CommunityId,
    handle: &str,
) -> Result<Option<UserGroupRecord>> {
    let row = sqlx::query(
        "SELECT id, handle, name, description, created_by, created_at, updated_at, deleted_at \
         FROM user_groups \
         WHERE community_id = $1 AND handle = $2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(handle)
    .fetch_optional(pool)
    .await?;

    row.map(row_to_group_record).transpose()
}

/// Lists all active user groups in a community, ordered by handle.
pub async fn list_groups(pool: &PgPool, community: CommunityId) -> Result<Vec<UserGroupRecord>> {
    let rows = sqlx::query(
        "SELECT id, handle, name, description, created_by, created_at, updated_at, deleted_at \
         FROM user_groups \
         WHERE community_id = $1 AND deleted_at IS NULL \
         ORDER BY handle ASC, id ASC",
    )
    .bind(community.as_uuid())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(row_to_group_record).collect()
}

/// Updates group metadata and optionally replaces the entire default-channel list.
///
/// At least one metadata field or `default_channels` replacement must be
/// supplied. Metadata and default-channel replacement commit atomically.
pub async fn update_group(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
    updates: UserGroupUpdate,
) -> Result<UserGroupRecord> {
    if updates.handle.is_none()
        && updates.name.is_none()
        && updates.description.is_none()
        && updates.default_channels.is_none()
    {
        return Err(DbError::InvalidData(
            "at least one field must be provided for user group update".to_owned(),
        ));
    }
    if updates
        .handle
        .as_deref()
        .is_some_and(|handle| !buzz_core::user_group::is_valid_group_handle(handle))
    {
        return Err(DbError::InvalidData(
            "user group handle must match ^[a-z0-9][a-z0-9_-]{1,31}$".to_owned(),
        ));
    }

    let description_changed = updates.description.is_some();
    let description = updates
        .description
        .as_ref()
        .and_then(|value| value.as_deref());
    let conflict_handle = updates.handle.as_deref().unwrap_or_default();

    let mut tx = pool.begin().await?;
    let row = match sqlx::query(
        "UPDATE user_groups SET \
             handle = COALESCE($1, handle), \
             name = COALESCE($2, name), \
             description = CASE WHEN $3 THEN $4 ELSE description END, \
             updated_at = now() \
         WHERE community_id = $5 AND id = $6 AND deleted_at IS NULL \
         RETURNING id, handle, name, description, created_by, created_at, updated_at, deleted_at",
    )
    .bind(updates.handle.as_deref())
    .bind(updates.name.as_deref())
    .bind(description_changed)
    .bind(description)
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return Err(DbError::UserGroupNotFound(group_id)),
        Err(error) => return Err(map_handle_conflict(error, conflict_handle)),
    };

    if let Some(default_channels) = updates.default_channels.as_deref() {
        sqlx::query(
            "DELETE FROM user_group_default_channels \
             WHERE community_id = $1 AND group_id = $2",
        )
        .bind(community.as_uuid())
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
        insert_default_channels_tx(&mut tx, community, group_id, default_channels).await?;
    }

    let record = row_to_group_record(row)?;
    tx.commit().await?;
    Ok(record)
}

/// Soft-deletes a user group.
///
/// Returns `true` when an active row was deleted and `false` when the group was
/// already deleted or absent.
pub async fn soft_delete_group(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE user_groups \
         SET deleted_at = now(), updated_at = now() \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Adds members to an active user group.
///
/// Existing memberships are left unchanged. The result identifies the newly
/// inserted pubkeys and the default channels observed while the group row was
/// locked, so callers can apply default-channel side effects without races.
pub async fn add_members(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
    pubkeys: &[String],
    added_by: &str,
) -> Result<AddMembersResult> {
    let mut tx = pool.begin().await?;
    lock_active_group(&mut tx, community, group_id).await?;
    let mut added_pubkeys =
        insert_members_tx(&mut tx, community, group_id, pubkeys, added_by).await?;
    if !added_pubkeys.is_empty() {
        touch_group_tx(&mut tx, community, group_id).await?;
    }
    let default_channels = list_default_channels_tx(&mut tx, community, group_id).await?;
    tx.commit().await?;
    added_pubkeys.sort_unstable();
    Ok(AddMembersResult {
        added_pubkeys,
        default_channels,
    })
}

/// Removes members from an active user group.
///
/// Missing memberships are ignored. Returns the number of removed rows.
pub async fn remove_members(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
    pubkeys: &[String],
) -> Result<u64> {
    if pubkeys.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    lock_active_group(&mut tx, community, group_id).await?;
    let result = sqlx::query(
        "DELETE FROM user_group_members \
         WHERE community_id = $1 AND group_id = $2 AND pubkey = ANY($3)",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .bind(pubkeys)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() > 0 {
        touch_group_tx(&mut tx, community, group_id).await?;
    }
    tx.commit().await?;
    Ok(result.rows_affected())
}

/// Lists members of an active user group, ordered by insertion time and pubkey.
pub async fn list_members(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
) -> Result<Vec<UserGroupMemberRecord>> {
    let rows = sqlx::query(
        "SELECT members.group_id, members.pubkey, members.added_by, members.added_at \
         FROM user_group_members members \
         JOIN user_groups groups \
           ON groups.community_id = members.community_id \
          AND groups.id = members.group_id \
          AND groups.deleted_at IS NULL \
         WHERE members.community_id = $1 AND members.group_id = $2 \
         ORDER BY members.added_at ASC, members.pubkey ASC",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        get_group_by_id(pool, community, group_id).await?;
    }
    rows.into_iter().map(row_to_member_record).collect()
}

/// Lists the full default-channel list for an active user group.
pub async fn list_default_channels(
    pool: &PgPool,
    community: CommunityId,
    group_id: Uuid,
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query(
        "SELECT defaults.channel_id \
         FROM user_group_default_channels defaults \
         JOIN user_groups groups \
           ON groups.community_id = defaults.community_id \
          AND groups.id = defaults.group_id \
          AND groups.deleted_at IS NULL \
         WHERE defaults.community_id = $1 AND defaults.group_id = $2 \
         ORDER BY defaults.channel_id ASC",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        get_group_by_id(pool, community, group_id).await?;
    }
    rows.into_iter()
        .map(|row| row.try_get("channel_id").map_err(DbError::from))
        .collect()
}

fn row_to_group_record(row: sqlx::postgres::PgRow) -> Result<UserGroupRecord> {
    Ok(UserGroupRecord {
        id: row.try_get("id")?,
        handle: row.try_get("handle")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        created_by: row.try_get("created_by")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        deleted_at: row.try_get("deleted_at")?,
    })
}

fn row_to_member_record(row: sqlx::postgres::PgRow) -> Result<UserGroupMemberRecord> {
    Ok(UserGroupMemberRecord {
        group_id: row.try_get("group_id")?,
        pubkey: row.try_get("pubkey")?,
        added_by: row.try_get("added_by")?,
        added_at: row.try_get("added_at")?,
    })
}

fn map_handle_conflict(error: sqlx::Error, handle: &str) -> DbError {
    if matches!(
        &error,
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some("23505")
                && database_error.constraint() == Some(ACTIVE_HANDLE_INDEX)
    ) {
        DbError::UserGroupHandleConflict(handle.to_owned())
    } else {
        error.into()
    }
}

async fn lock_active_group(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    group_id: Uuid,
) -> Result<()> {
    let exists = sqlx::query(
        "SELECT 1 FROM user_groups \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;
    if exists.is_none() {
        return Err(DbError::UserGroupNotFound(group_id));
    }
    Ok(())
}

async fn insert_members_tx(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    group_id: Uuid,
    pubkeys: &[String],
    added_by: &str,
) -> Result<Vec<String>> {
    if pubkeys.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Postgres>::new(
        "INSERT INTO user_group_members \
         (community_id, group_id, pubkey, added_by) ",
    );
    query.push_values(pubkeys, |mut row, pubkey| {
        row.push_bind(community.as_uuid())
            .push_bind(group_id)
            .push_bind(pubkey)
            .push_bind(added_by);
    });
    query.push(
        " ON CONFLICT (community_id, group_id, pubkey) DO NOTHING \
         RETURNING pubkey",
    );
    let rows = query.build().fetch_all(&mut **tx).await?;
    rows.into_iter()
        .map(|row| row.try_get("pubkey").map_err(DbError::from))
        .collect()
}

async fn insert_default_channels_tx(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    group_id: Uuid,
    channel_ids: &[Uuid],
) -> Result<u64> {
    if channel_ids.is_empty() {
        return Ok(0);
    }

    let mut query = QueryBuilder::<Postgres>::new(
        "INSERT INTO user_group_default_channels \
         (community_id, group_id, channel_id) ",
    );
    query.push_values(channel_ids, |mut row, channel_id| {
        row.push_bind(community.as_uuid())
            .push_bind(group_id)
            .push_bind(channel_id);
    });
    query.push(" ON CONFLICT (community_id, group_id, channel_id) DO NOTHING");
    let result = query.build().execute(&mut **tx).await?;
    Ok(result.rows_affected())
}

async fn touch_group_tx(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    group_id: Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE user_groups SET updated_at = now() \
         WHERE community_id = $1 AND id = $2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn list_default_channels_tx(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    group_id: Uuid,
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query(
        "SELECT channel_id FROM user_group_default_channels \
         WHERE community_id = $1 AND group_id = $2 \
         ORDER BY channel_id ASC",
    )
    .bind(community.as_uuid())
    .bind(group_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| row.try_get("channel_id").map_err(DbError::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("user-group-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    async fn make_channel(pool: &PgPool, community: CommunityId) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO channels \
             (community_id, id, name, channel_type, visibility, created_by) \
             VALUES ($1, $2, $3, 'stream', 'open', $4)",
        )
        .bind(community.as_uuid())
        .bind(id)
        .bind(format!("group-default-{}", id.simple()))
        .bind(vec![1_u8; 32])
        .execute(pool)
        .await
        .expect("insert test channel");
        id
    }

    fn pubkey(marker: char) -> String {
        marker.to_string().repeat(64)
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn group_crud_members_defaults_conflict_and_soft_delete() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let community = make_community(&pool).await;
        let channel_a = make_channel(&pool, community).await;
        let channel_b = make_channel(&pool, community).await;
        let creator = pubkey('a');
        let initial = vec![pubkey('b')];
        let group_id = Uuid::new_v4();

        let created = create_group(
            &pool,
            community,
            group_id,
            "ios-team",
            "iOS Team",
            Some("Mobile engineers"),
            &creator,
            &initial,
            &[channel_a],
        )
        .await
        .expect("create group");
        assert_eq!(created.handle, "ios-team");
        assert_eq!(
            get_group_by_handle(&pool, community, "ios-team")
                .await
                .expect("get by handle")
                .expect("active group")
                .id,
            group_id
        );

        let conflict = create_group(
            &pool,
            community,
            Uuid::new_v4(),
            "ios-team",
            "Other iOS Team",
            None,
            &creator,
            &[],
            &[],
        )
        .await
        .expect_err("duplicate active handle must conflict");
        assert!(matches!(
            conflict,
            DbError::UserGroupHandleConflict(ref handle) if handle == "ios-team"
        ));

        create_group(
            &pool,
            community,
            Uuid::new_v4(),
            "other-team",
            "Other Team",
            None,
            &creator,
            &[],
            &[channel_b],
        )
        .await
        .expect("create rival group");
        let rename_conflict = update_group(
            &pool,
            community,
            group_id,
            UserGroupUpdate {
                handle: Some("other-team".to_owned()),
                name: Some("Must Roll Back".to_owned()),
                default_channels: Some(vec![channel_b]),
                ..UserGroupUpdate::default()
            },
        )
        .await
        .expect_err("rename to active handle must conflict");
        assert!(matches!(
            rename_conflict,
            DbError::UserGroupHandleConflict(ref handle) if handle == "other-team"
        ));
        assert_eq!(
            get_group_by_id(&pool, community, group_id)
                .await
                .expect("group survives failed rename")
                .name,
            "iOS Team"
        );
        assert_eq!(
            list_default_channels(&pool, community, group_id)
                .await
                .expect("defaults survive failed rename"),
            vec![channel_a]
        );

        let added = add_members(
            &pool,
            community,
            group_id,
            &[pubkey('c'), pubkey('d')],
            &creator,
        )
        .await
        .expect("add members");
        assert_eq!(added.added_pubkeys, vec![pubkey('c'), pubkey('d')]);
        assert_eq!(added.default_channels, vec![channel_a]);
        assert_eq!(
            remove_members(&pool, community, group_id, &[pubkey('b')])
                .await
                .expect("remove member"),
            1
        );
        assert_eq!(
            list_members(&pool, community, group_id)
                .await
                .expect("list members")
                .len(),
            2
        );

        update_group(
            &pool,
            community,
            group_id,
            UserGroupUpdate {
                name: Some("Apple Platforms".to_owned()),
                description: Some(None),
                default_channels: Some(vec![channel_b]),
                ..UserGroupUpdate::default()
            },
        )
        .await
        .expect("update group");
        assert_eq!(
            list_default_channels(&pool, community, group_id)
                .await
                .expect("list default channels"),
            vec![channel_b]
        );
        update_group(
            &pool,
            community,
            group_id,
            UserGroupUpdate {
                default_channels: Some(Vec::new()),
                ..UserGroupUpdate::default()
            },
        )
        .await
        .expect("clear default channels");
        assert!(list_default_channels(&pool, community, group_id)
            .await
            .expect("list cleared default channels")
            .is_empty());

        assert!(soft_delete_group(&pool, community, group_id)
            .await
            .expect("soft delete"));
        assert!(matches!(
            get_group_by_id(&pool, community, group_id).await,
            Err(DbError::UserGroupNotFound(id)) if id == group_id
        ));

        create_group(
            &pool,
            community,
            Uuid::new_v4(),
            "ios-team",
            "Recreated iOS Team",
            None,
            &creator,
            &[],
            &[],
        )
        .await
        .expect("soft deletion releases handle");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn groups_are_confined_to_their_community() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let shared_id = Uuid::new_v4();
        let creator = pubkey('e');

        for community in [community_a, community_b] {
            create_group(
                &pool,
                community,
                shared_id,
                "shared-handle",
                "Shared",
                None,
                &creator,
                &[],
                &[],
            )
            .await
            .expect("same id and handle may exist in separate communities");
        }

        assert_eq!(
            list_groups(&pool, community_a)
                .await
                .expect("list community A")
                .iter()
                .filter(|group| group.id == shared_id)
                .count(),
            1
        );
        assert_eq!(
            list_groups(&pool, community_b)
                .await
                .expect("list community B")
                .iter()
                .filter(|group| group.id == shared_id)
                .count(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cross_community_default_channel_failure_rolls_back_parent_and_update() {
        let pool = setup_pool().await;
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;
        let channel_a = make_channel(&pool, community_a).await;
        let channel_b = make_channel(&pool, community_b).await;
        let creator = pubkey('f');
        let failed_group_id = Uuid::new_v4();

        let create_error = create_group(
            &pool,
            community_a,
            failed_group_id,
            "foreign-default",
            "Foreign Default",
            None,
            &creator,
            &[pubkey('1')],
            &[channel_b],
        )
        .await
        .expect_err("cross-community default channel must fail");
        assert!(matches!(create_error, DbError::Sqlx(_)));
        assert!(matches!(
            get_group_by_id(&pool, community_a, failed_group_id).await,
            Err(DbError::UserGroupNotFound(id)) if id == failed_group_id
        ));

        let group_id = Uuid::new_v4();
        create_group(
            &pool,
            community_a,
            group_id,
            "local-default",
            "Original Name",
            None,
            &creator,
            &[],
            &[channel_a],
        )
        .await
        .expect("create local group");
        let update_error = update_group(
            &pool,
            community_a,
            group_id,
            UserGroupUpdate {
                name: Some("Must Roll Back".to_owned()),
                default_channels: Some(vec![channel_b]),
                ..UserGroupUpdate::default()
            },
        )
        .await
        .expect_err("cross-community replacement must fail");
        assert!(matches!(update_error, DbError::Sqlx(_)));
        assert_eq!(
            get_group_by_id(&pool, community_a, group_id)
                .await
                .expect("group survives failed update")
                .name,
            "Original Name"
        );
        assert_eq!(
            list_default_channels(&pool, community_a, group_id)
                .await
                .expect("defaults survive failed update"),
            vec![channel_a]
        );
    }
}
