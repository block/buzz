//! SQLite storage for the single-node community profile.
//!
//! This intentionally covers only community bootstrap, channels, memberships,
//! and relay ownership. Event and user persistence lands with PR 2.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::{CommunityRecord, EnsuredCommunityRecord, Result};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS communities (
    id TEXT PRIMARY KEY NOT NULL,
    host TEXT NOT NULL COLLATE NOCASE UNIQUE,
    icon TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    archived_at INTEGER
);

CREATE TABLE IF NOT EXISTS relay_members (
    community_id TEXT NOT NULL,
    pubkey TEXT NOT NULL COLLATE NOCASE,
    role TEXT NOT NULL,
    added_by TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (community_id, pubkey),
    FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY NOT NULL,
    community_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    canvas TEXT,
    channel_type TEXT NOT NULL,
    visibility TEXT NOT NULL,
    participant_hash BLOB,
    created_by BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    archived_at INTEGER,
    deleted_at INTEGER,
    nip29_group_id TEXT,
    topic_required INTEGER NOT NULL DEFAULT 0,
    max_members INTEGER,
    topic TEXT,
    topic_set_by BLOB,
    topic_set_at INTEGER,
    purpose TEXT,
    purpose_set_by BLOB,
    purpose_set_at INTEGER,
    ttl_seconds INTEGER,
    ttl_deadline INTEGER,
    FOREIGN KEY (community_id) REFERENCES communities(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS channel_members (
    channel_id TEXT NOT NULL,
    pubkey BLOB NOT NULL,
    role TEXT NOT NULL,
    joined_at INTEGER NOT NULL DEFAULT (unixepoch()),
    invited_by BLOB,
    hidden_at INTEGER,
    removed_at INTEGER,
    PRIMARY KEY (channel_id, pubkey),
    FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
);
"#;

pub(crate) async fn connect(path_or_url: &str) -> Result<SqlitePool> {
    let database_url = if path_or_url.starts_with("sqlite:") {
        path_or_url.to_owned()
    } else {
        format!("sqlite://{path_or_url}")
    };
    let options = SqliteConnectOptions::from_str(&database_url)?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub(crate) async fn migrate(pool: &SqlitePool) -> Result<()> {
    for statement in SCHEMA.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::query(statement).execute(pool).await?;
    }
    Ok(())
}

pub(crate) async fn lookup_community_by_host(
    pool: &SqlitePool,
    normalized_host: &str,
) -> Result<Option<CommunityRecord>> {
    let row = sqlx::query(
        "SELECT id, host FROM communities WHERE host = ?1 COLLATE NOCASE AND archived_at IS NULL",
    )
    .bind(normalized_host)
    .fetch_optional(pool)
    .await?;
    row.map(community_record).transpose()
}

pub(crate) async fn lookup_community_host(
    pool: &SqlitePool,
    community_id: CommunityId,
) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT host FROM communities WHERE id = ?1 AND archived_at IS NULL")
            .bind(community_id.as_uuid().to_string())
            .fetch_optional(pool)
            .await?,
    )
}

pub(crate) async fn is_community_active(
    pool: &SqlitePool,
    community_id: CommunityId,
) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM communities WHERE id = ?1 AND archived_at IS NULL",
    )
    .bind(community_id.as_uuid().to_string())
    .fetch_one(pool)
    .await?;
    Ok(count != 0)
}

pub(crate) async fn rebind_single_node_community_host(
    pool: &SqlitePool,
    normalized_host: &str,
    owner_pubkey: &str,
) -> Result<Option<CommunityRecord>> {
    let mut tx = pool.begin().await?;

    // Defect-era databases may contain several loopback communities, one per
    // ephemeral port. Prefer a community owned by this desktop identity, then
    // the oldest community. This keeps ownership stable without depending on
    // event persistence, which arrives with PR 2.
    let row = sqlx::query(
        r#"SELECT c.id, c.host
           FROM communities c
           LEFT JOIN relay_members rm
             ON rm.community_id = c.id
            AND lower(rm.pubkey) = lower(?1)
            AND rm.role = 'owner'
           WHERE c.archived_at IS NULL
             AND c.host LIKE '127.0.0.1:%'
           ORDER BY CASE WHEN rm.pubkey IS NULL THEN 0 ELSE 1 END DESC, c.created_at ASC, c.id ASC
           LIMIT 1"#,
    )
    .bind(owner_pubkey)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    let record = community_record(row)?;
    if record.host.eq_ignore_ascii_case(normalized_host) {
        tx.commit().await?;
        return Ok(Some(record));
    }

    // The OS may reuse a port that belongs to another stale defect-era row.
    // Swap that collision to the selected row's old authority transactionally,
    // using a temporary unique host to satisfy the case-insensitive constraint.
    if let Some(collision_id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM communities WHERE host = ?1 COLLATE NOCASE AND id <> ?2",
    )
    .bind(normalized_host)
    .bind(record.id.as_uuid().to_string())
    .fetch_optional(&mut *tx)
    .await?
    {
        let temporary_host = format!("rebind-{}.invalid", Uuid::new_v4());
        sqlx::query("UPDATE communities SET host = ?2 WHERE id = ?1")
            .bind(&collision_id)
            .bind(&temporary_host)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE communities SET host = ?2 WHERE id = ?1")
            .bind(record.id.as_uuid().to_string())
            .bind(normalized_host)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE communities SET host = ?2 WHERE id = ?1")
            .bind(collision_id)
            .bind(&record.host)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE communities SET host = ?2 WHERE id = ?1")
            .bind(record.id.as_uuid().to_string())
            .bind(normalized_host)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Some(CommunityRecord {
        id: record.id,
        host: normalized_host.to_string(),
    }))
}

pub(crate) async fn ensure_configured_community(
    pool: &SqlitePool,
    normalized_host: &str,
) -> Result<EnsuredCommunityRecord> {
    let id = Uuid::new_v4();
    let inserted =
        sqlx::query("INSERT INTO communities (id, host) VALUES (?1, ?2) ON CONFLICT DO NOTHING")
            .bind(id.to_string())
            .bind(normalized_host)
            .execute(pool)
            .await?
            .rows_affected()
            == 1;

    let row = sqlx::query("SELECT id, host FROM communities WHERE host = ?1 COLLATE NOCASE")
        .bind(normalized_host)
        .fetch_one(pool)
        .await?;
    let record = community_record(row)?;
    Ok(EnsuredCommunityRecord {
        id: record.id,
        host: record.host,
        created: inserted,
    })
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_channel_with_id(
    pool: &SqlitePool,
    community: CommunityId,
    channel_id: Uuid,
    name: &str,
    channel_type: crate::channel::ChannelType,
    visibility: crate::channel::ChannelVisibility,
    description: Option<&str>,
    created_by: &[u8],
    ttl_seconds: Option<i32>,
) -> Result<(crate::channel::ChannelRecord, bool)> {
    if created_by.len() != 32 {
        return Err(crate::DbError::InvalidData(format!(
            "pubkey must be 32 bytes, got {}",
            created_by.len()
        )));
    }
    if channel_id.is_nil() {
        return Err(crate::DbError::InvalidData(
            "channel_id must not be nil (reserved for global fan-out)".into(),
        ));
    }
    let name = buzz_core::channel::canonical_channel_name(name);
    if name.trim().is_empty() {
        return Err(crate::DbError::InvalidData(
            "channel name is required".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, CASE WHEN ?8 IS NULL THEN NULL ELSE unixepoch() + ?8 END) ON CONFLICT DO NOTHING",
    )
    .bind(channel_id.to_string()).bind(community.as_uuid().to_string()).bind(name)
    .bind(channel_type.as_str()).bind(visibility.as_str()).bind(description).bind(created_by)
    .bind(ttl_seconds).execute(&mut *tx).await?.rows_affected() == 1;
    if inserted {
        sqlx::query("INSERT INTO channel_members (channel_id, pubkey, role, invited_by) VALUES (?1, ?2, 'owner', ?2)")
            .bind(channel_id.to_string()).bind(created_by).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok((get_channel(pool, community, channel_id).await?, inserted))
}

pub(crate) async fn get_channel(
    pool: &SqlitePool,
    community: CommunityId,
    channel_id: Uuid,
) -> Result<crate::channel::ChannelRecord> {
    let row = sqlx::query(
        "SELECT * FROM channels WHERE community_id = ?1 AND id = ?2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid().to_string())
    .bind(channel_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or(crate::DbError::ChannelNotFound(channel_id))?;
    channel_record(row)
}

pub(crate) async fn add_member(
    pool: &SqlitePool,
    community: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
    role: crate::channel::MemberRole,
    invited_by: Option<&[u8]>,
) -> Result<crate::channel::MemberRecord> {
    if pubkey.len() != 32 {
        return Err(crate::DbError::InvalidData(format!(
            "pubkey must be 32 bytes, got {}",
            pubkey.len()
        )));
    }

    let mut tx = pool.begin().await?;
    let channel = sqlx::query(
        "SELECT * FROM channels WHERE community_id = ?1 AND id = ?2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid().to_string())
    .bind(channel_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(crate::DbError::ChannelNotFound(channel_id))?;
    let channel = channel_record(channel)?;

    let effective_role = if channel.visibility == "private" {
        let inviter = invited_by.ok_or_else(|| {
            crate::DbError::AccessDenied("private channel requires an invite".to_string())
        })?;
        let creator_bootstrap = inviter == pubkey && inviter == channel.created_by.as_slice();
        if !creator_bootstrap {
            let inviter_role = active_member_role(&mut tx, channel_id, inviter)
                .await?
                .ok_or_else(|| {
                    crate::DbError::AccessDenied("inviter is not an active member".to_string())
                })?;
            if role.is_elevated() && !is_elevated_role(&inviter_role) {
                return Err(crate::DbError::AccessDenied(
                    "only owners/admins may grant elevated roles".to_string(),
                ));
            }
        }
        role
    } else if role.is_elevated() {
        let granter_role = match invited_by {
            Some(inviter) => active_member_role(&mut tx, channel_id, inviter).await?,
            None => None,
        };
        if !granter_role.is_some_and(|role| is_elevated_role(&role)) {
            return Err(crate::DbError::AccessDenied(
                "only owners/admins may grant elevated roles".to_string(),
            ));
        }
        role
    } else {
        role
    };

    if let Some(current_role) = active_member_role(&mut tx, channel_id, pubkey).await? {
        if current_role != effective_role.as_str() {
            let actor_role = match invited_by {
                Some(inviter) => active_member_role(&mut tx, channel_id, inviter).await?,
                None => None,
            };
            if !actor_role.is_some_and(|role| is_elevated_role(&role)) {
                return Err(crate::DbError::AccessDenied(
                    "only owners/admins may change an active member's role".to_string(),
                ));
            }
            if current_role == "owner" && effective_role != crate::channel::MemberRole::Owner {
                let owner_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM channel_members WHERE channel_id = ?1 AND role = 'owner' AND removed_at IS NULL",
            )
            .bind(channel_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
                if owner_count <= 1 {
                    return Err(crate::DbError::AccessDenied(
                        "cannot demote the last owner — transfer ownership first".to_string(),
                    ));
                }
            }
        }
    }

    sqlx::query("INSERT INTO channel_members (channel_id, pubkey, role, invited_by) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(channel_id, pubkey) DO UPDATE SET role = excluded.role, removed_at = NULL")
        .bind(channel_id.to_string()).bind(pubkey).bind(effective_role.as_str()).bind(invited_by).execute(&mut *tx).await?;
    let row = sqlx::query("SELECT channel_id, pubkey, role, joined_at, invited_by, removed_at FROM channel_members WHERE channel_id = ?1 AND pubkey = ?2")
        .bind(channel_id.to_string()).bind(pubkey).fetch_one(&mut *tx).await?;
    let record = member_record(row)?;
    tx.commit().await?;
    Ok(record)
}

async fn active_member_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<Option<String>> {
    sqlx::query_scalar(
        "SELECT role FROM channel_members WHERE channel_id = ?1 AND pubkey = ?2 AND removed_at IS NULL",
    )
    .bind(channel_id.to_string())
    .bind(pubkey)
    .fetch_optional(&mut **tx)
    .await
    .map_err(Into::into)
}

fn is_elevated_role(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}

pub(crate) async fn is_member(
    pool: &SqlitePool,
    community: CommunityId,
    channel_id: Uuid,
    pubkey: &[u8],
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM channel_members cm JOIN channels c ON c.id = cm.channel_id WHERE c.community_id = ?1 AND c.id = ?2 AND cm.pubkey = ?3 AND cm.removed_at IS NULL AND c.deleted_at IS NULL")
        .bind(community.as_uuid().to_string()).bind(channel_id.to_string()).bind(pubkey).fetch_one(pool).await? != 0)
}

pub(crate) async fn get_members(
    pool: &SqlitePool,
    community: CommunityId,
    channel_id: Uuid,
) -> Result<Vec<crate::channel::MemberRecord>> {
    get_channel(pool, community, channel_id).await?;
    sqlx::query("SELECT channel_id, pubkey, role, joined_at, invited_by, removed_at FROM channel_members WHERE channel_id = ?1 AND removed_at IS NULL ORDER BY joined_at, pubkey")
        .bind(channel_id.to_string()).fetch_all(pool).await?.into_iter().map(member_record).collect()
}

pub(crate) async fn get_accessible_channel_ids(
    pool: &SqlitePool,
    community: CommunityId,
    pubkey: &[u8],
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query_scalar::<_, String>("SELECT c.id FROM channels c JOIN channel_members cm ON cm.channel_id = c.id WHERE c.community_id = ?1 AND cm.pubkey = ?2 AND cm.removed_at IS NULL AND c.deleted_at IS NULL ORDER BY c.created_at, c.id")
        .bind(community.as_uuid().to_string()).bind(pubkey).fetch_all(pool).await?;
    rows.into_iter()
        .map(|id| {
            Uuid::parse_str(&id)
                .map_err(|e| crate::DbError::InvalidData(format!("invalid SQLite channel id: {e}")))
        })
        .collect()
}

fn timestamp(value: i64) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp(value, 0).ok_or(crate::DbError::InvalidTimestamp(value))
}
fn optional_timestamp(value: Option<i64>) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    value.map(timestamp).transpose()
}
fn channel_record(row: sqlx::sqlite::SqliteRow) -> Result<crate::channel::ChannelRecord> {
    let id: String = row.try_get("id")?;
    Ok(crate::channel::ChannelRecord {
        id: Uuid::parse_str(&id)
            .map_err(|e| crate::DbError::InvalidData(format!("invalid SQLite channel id: {e}")))?,
        name: row.try_get("name")?,
        channel_type: row.try_get("channel_type")?,
        visibility: row.try_get("visibility")?,
        description: row.try_get("description")?,
        canvas: row.try_get("canvas")?,
        created_by: row.try_get("created_by")?,
        created_at: timestamp(row.try_get("created_at")?)?,
        updated_at: timestamp(row.try_get("updated_at")?)?,
        archived_at: optional_timestamp(row.try_get("archived_at")?)?,
        deleted_at: optional_timestamp(row.try_get("deleted_at")?)?,
        nip29_group_id: row.try_get("nip29_group_id")?,
        topic_required: row.try_get::<i64, _>("topic_required")? != 0,
        max_members: row.try_get("max_members")?,
        topic: row.try_get("topic")?,
        topic_set_by: row.try_get("topic_set_by")?,
        topic_set_at: optional_timestamp(row.try_get("topic_set_at")?)?,
        purpose: row.try_get("purpose")?,
        purpose_set_by: row.try_get("purpose_set_by")?,
        purpose_set_at: optional_timestamp(row.try_get("purpose_set_at")?)?,
        ttl_seconds: row.try_get("ttl_seconds")?,
        ttl_deadline: optional_timestamp(row.try_get("ttl_deadline")?)?,
    })
}
fn member_record(row: sqlx::sqlite::SqliteRow) -> Result<crate::channel::MemberRecord> {
    let id: String = row.try_get("channel_id")?;
    Ok(crate::channel::MemberRecord {
        channel_id: Uuid::parse_str(&id)
            .map_err(|e| crate::DbError::InvalidData(format!("invalid SQLite channel id: {e}")))?,
        pubkey: row.try_get("pubkey")?,
        role: row.try_get("role")?,
        joined_at: timestamp(row.try_get("joined_at")?)?,
        invited_by: row.try_get("invited_by")?,
        removed_at: optional_timestamp(row.try_get("removed_at")?)?,
    })
}
pub(crate) async fn is_relay_member(
    pool: &SqlitePool,
    community: CommunityId,
    pubkey: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM relay_members WHERE community_id = ?1 AND pubkey = ?2 COLLATE NOCASE",
    )
    .bind(community.as_uuid().to_string())
    .bind(pubkey)
    .fetch_one(pool)
    .await?
        != 0)
}

pub(crate) async fn get_relay_member(
    pool: &SqlitePool,
    community: CommunityId,
    pubkey: &str,
) -> Result<Option<crate::relay_members::RelayMember>> {
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at FROM relay_members WHERE community_id = ?1 AND pubkey = ?2 COLLATE NOCASE",
    )
    .bind(community.as_uuid().to_string())
    .bind(pubkey)
    .fetch_optional(pool)
    .await?;
    row.map(relay_member).transpose()
}

pub(crate) async fn list_relay_members(
    pool: &SqlitePool,
    community: CommunityId,
) -> Result<Vec<crate::relay_members::RelayMember>> {
    sqlx::query(
        "SELECT pubkey, role, added_by, created_at, updated_at FROM relay_members WHERE community_id = ?1 ORDER BY created_at ASC, pubkey ASC",
    )
    .bind(community.as_uuid().to_string())
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(relay_member)
    .collect()
}

pub(crate) async fn add_relay_member(
    pool: &SqlitePool,
    community: CommunityId,
    pubkey: &str,
    role: &str,
    added_by: Option<&str>,
) -> Result<bool> {
    Ok(sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) VALUES (?1, lower(?2), ?3, ?4) ON CONFLICT DO NOTHING",
    )
    .bind(community.as_uuid().to_string())
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(pool)
    .await?
    .rows_affected() == 1)
}

pub(crate) async fn bootstrap_owner(
    pool: &SqlitePool,
    community: CommunityId,
    owner_pubkey: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE relay_members SET role = 'admin', updated_at = unixepoch() WHERE community_id = ?1 AND role = 'owner' AND pubkey <> ?2 COLLATE NOCASE",
    )
    .bind(community.as_uuid().to_string())
    .bind(owner_pubkey)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role) VALUES (?1, lower(?2), 'owner') ON CONFLICT(community_id, pubkey) DO UPDATE SET role = 'owner', updated_at = unixepoch()",
    )
    .bind(community.as_uuid().to_string())
    .bind(owner_pubkey)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn relay_member(row: sqlx::sqlite::SqliteRow) -> Result<crate::relay_members::RelayMember> {
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    Ok(crate::relay_members::RelayMember {
        pubkey: row.try_get("pubkey")?,
        role: row.try_get("role")?,
        added_by: row.try_get("added_by")?,
        created_at: chrono::DateTime::from_timestamp(created_at, 0)
            .ok_or(crate::DbError::InvalidTimestamp(created_at))?,
        updated_at: chrono::DateTime::from_timestamp(updated_at, 0)
            .ok_or(crate::DbError::InvalidTimestamp(updated_at))?,
    })
}

fn community_record(row: sqlx::sqlite::SqliteRow) -> Result<CommunityRecord> {
    let id: String = row.try_get("id")?;
    let id = Uuid::parse_str(&id)
        .map_err(|e| crate::DbError::InvalidData(format!("invalid SQLite community id: {e}")))?;
    Ok(CommunityRecord {
        id: CommunityId::from_uuid(id),
        host: row.try_get("host")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    #[tokio::test]
    async fn community_slice_survives_temporary_file_reopen() {
        let path = std::env::temp_dir().join(format!("buzz-db-{}.sqlite", Uuid::new_v4()));
        let path_string = path.to_string_lossy().into_owned();
        let owner = Keys::generate();
        let owner_bytes = owner.public_key().to_bytes();
        let owner_hex = owner.public_key().to_hex();
        let channel_id = Uuid::new_v4();

        let pool = connect(&path_string).await.unwrap();
        let ensured = ensure_configured_community(&pool, "Local.Buzz")
            .await
            .unwrap();
        bootstrap_owner(&pool, ensured.id, &owner_hex)
            .await
            .unwrap();
        let (channel, created) = create_channel_with_id(
            &pool,
            ensured.id,
            channel_id,
            "general",
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Private,
            None,
            &owner_bytes,
            None,
        )
        .await
        .unwrap();
        assert!(created);
        assert_eq!(channel.id, channel_id);
        assert!(is_member(&pool, ensured.id, channel_id, &owner_bytes)
            .await
            .unwrap());
        pool.close().await;

        let reopened = connect(&path_string).await.unwrap();
        let found = lookup_community_by_host(&reopened, "local.buzz")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, ensured.id);
        assert!(is_community_active(&reopened, ensured.id).await.unwrap());
        assert!(is_relay_member(&reopened, ensured.id, &owner_hex)
            .await
            .unwrap());
        assert_eq!(
            get_accessible_channel_ids(&reopened, ensured.id, &owner_bytes)
                .await
                .unwrap(),
            vec![channel_id]
        );
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn sqlite_membership_preserves_public_contract_invariants() {
        let pool = connect(":memory:").await.unwrap();
        let community = ensure_configured_community(&pool, "local.buzz")
            .await
            .unwrap();
        let owner = Keys::generate().public_key().to_bytes();
        let member = Keys::generate().public_key().to_bytes();
        let admin = Keys::generate().public_key().to_bytes();
        let other = Keys::generate().public_key().to_bytes();
        let channel_id = Uuid::new_v4();
        create_channel_with_id(
            &pool,
            community.id,
            channel_id,
            "private",
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Private,
            None,
            &owner,
            None,
        )
        .await
        .unwrap();

        assert!(matches!(
            add_member(
                &pool,
                community.id,
                channel_id,
                &[0; 31],
                crate::channel::MemberRole::Member,
                Some(&owner),
            )
            .await,
            Err(crate::DbError::InvalidData(_))
        ));
        assert!(matches!(
            add_member(
                &pool,
                community.id,
                channel_id,
                &member,
                crate::channel::MemberRole::Member,
                None,
            )
            .await,
            Err(crate::DbError::AccessDenied(_))
        ));
        add_member(
            &pool,
            community.id,
            channel_id,
            &member,
            crate::channel::MemberRole::Member,
            Some(&owner),
        )
        .await
        .unwrap();
        add_member(
            &pool,
            community.id,
            channel_id,
            &admin,
            crate::channel::MemberRole::Admin,
            Some(&owner),
        )
        .await
        .unwrap();
        let readded = add_member(
            &pool,
            community.id,
            channel_id,
            &member,
            crate::channel::MemberRole::Member,
            Some(&admin),
        )
        .await
        .unwrap();
        assert_eq!(readded.invited_by, Some(owner.to_vec()));
        sqlx::query::<sqlx::Sqlite>(
            "UPDATE channel_members SET removed_at = unixepoch() WHERE channel_id = ?1 AND pubkey = ?2",
        )
            .bind(channel_id.to_string())
            .bind(&member[..])
            .execute(&pool)
            .await
            .unwrap();
        let reactivated = add_member(
            &pool,
            community.id,
            channel_id,
            &member,
            crate::channel::MemberRole::Member,
            Some(&admin),
        )
        .await
        .unwrap();
        assert_eq!(reactivated.invited_by, Some(owner.to_vec()));
        assert!(reactivated.removed_at.is_none());
        assert!(matches!(
            add_member(
                &pool,
                community.id,
                channel_id,
                &other,
                crate::channel::MemberRole::Admin,
                Some(&member),
            )
            .await,
            Err(crate::DbError::AccessDenied(_))
        ));
        assert!(matches!(
            add_member(
                &pool,
                community.id,
                channel_id,
                &member,
                crate::channel::MemberRole::Guest,
                Some(&member),
            )
            .await,
            Err(crate::DbError::AccessDenied(_))
        ));
        assert!(matches!(
            add_member(
                &pool,
                community.id,
                channel_id,
                &owner,
                crate::channel::MemberRole::Member,
                Some(&owner),
            )
            .await,
            Err(crate::DbError::AccessDenied(_))
        ));
    }

    #[tokio::test]
    async fn rebind_prefers_owned_then_oldest_community() {
        let pool = connect(":memory:").await.unwrap();
        let first = ensure_configured_community(&pool, "127.0.0.1:4000")
            .await
            .unwrap();
        let second = ensure_configured_community(&pool, "127.0.0.1:4001")
            .await
            .unwrap();
        bootstrap_owner(&pool, second.id, "owner").await.unwrap();
        let rebound = rebind_single_node_community_host(&pool, "127.0.0.1:5000", "owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.id, second.id);
        assert_eq!(
            lookup_community_host(&pool, first.id)
                .await
                .unwrap()
                .unwrap(),
            "127.0.0.1:4000"
        );
        assert_eq!(rebound.host, "127.0.0.1:5000");
    }
}
