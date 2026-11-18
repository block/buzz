//! In-transaction parent checks for the community, channel, user,
//! relay-member and workflow references that used to be foreign keys.
//!
//! Each helper locks the parent row `FOR KEY SHARE`: the lock a foreign key's
//! check takes, so a concurrent delete of the parent (which locks it
//! `FOR UPDATE` first) either waits for the writing transaction or runs first
//! and makes the check fail. While the foreign keys still exist the two checks
//! are idempotent; once they are dropped these checks are the only guard.
//!
//! A missing parent is `DbError::NotFound`, never a silent success. Every
//! lookup below the community is scoped by `community_id`, so a parent that
//! exists only in another community is missing.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::CommunityId;

/// Lock `communities (id)` `FOR KEY SHARE`, or fail with `NotFound`.
///
/// Any lifecycle state satisfies the reference, as it did under the foreign
/// key: a tombstoned community keeps its row. Writers that must refuse a
/// community being deleted use write admission instead.
pub(crate) async fn require_community(
    conn: &mut PgConnection,
    community_id: CommunityId,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>("SELECT 1 FROM communities WHERE id = $1 FOR KEY SHARE")
        .bind(community_id.as_uuid())
        .fetch_optional(conn)
        .await?
        .map(|_| ())
        .ok_or_else(|| DbError::NotFound(format!("community {community_id}")))
}

/// Lock `channels (community_id, id)` `FOR KEY SHARE`, or fail with `NotFound`.
///
/// A soft-deleted channel keeps its row and still satisfies a reference, as it
/// did under the foreign key.
pub(crate) async fn require_channel(
    conn: &mut PgConnection,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 FOR KEY SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_optional(conn)
    .await?
    .map(|_| ())
    .ok_or_else(|| DbError::NotFound(format!("channel {channel_id}")))
}

/// [`require_channel`] for a nullable reference: `None` references nothing.
pub(crate) async fn require_optional_channel(
    conn: &mut PgConnection,
    community_id: CommunityId,
    channel_id: Option<Uuid>,
) -> Result<()> {
    match channel_id {
        Some(channel_id) => require_channel(conn, community_id, channel_id).await,
        None => Ok(()),
    }
}

/// Lock `users (community_id, pubkey)` `FOR KEY SHARE`, or fail with
/// `NotFound(what)`.
pub(crate) async fn require_user(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &[u8],
    what: &str,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM users WHERE community_id = $1 AND pubkey = $2 FOR KEY SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(conn)
    .await?
    .map(|_| ())
    .ok_or_else(|| DbError::NotFound(what.to_owned()))
}

/// Lock `relay_members (community_id, pubkey)` `FOR KEY SHARE`, or fail with
/// `NotFound`.
///
/// Member removal locks the row `FOR UPDATE`, deletes the member's join-policy
/// acceptances, then the member, so an acceptance written under this lock is
/// either deleted with the member or refused.
pub(crate) async fn require_relay_member(
    conn: &mut PgConnection,
    community_id: CommunityId,
    pubkey: &str,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2 FOR KEY SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(pubkey)
    .fetch_optional(conn)
    .await?
    .map(|_| ())
    .ok_or_else(|| DbError::NotFound(format!("relay member {pubkey}")))
}

/// Lock `workflows (community_id, id)` `FOR KEY SHARE`, or fail with `NotFound`.
pub(crate) async fn require_workflow(
    conn: &mut PgConnection,
    community_id: CommunityId,
    workflow_id: Uuid,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM workflows WHERE community_id = $1 AND id = $2 FOR KEY SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(workflow_id)
    .fetch_optional(conn)
    .await?
    .map(|_| ())
    .ok_or_else(|| DbError::NotFound(format!("workflow {workflow_id}")))
}

/// Lock `workflow_runs (community_id, id)` `FOR KEY SHARE`, or fail with `NotFound`.
pub(crate) async fn require_workflow_run(
    conn: &mut PgConnection,
    community_id: CommunityId,
    run_id: Uuid,
) -> Result<()> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM workflow_runs WHERE community_id = $1 AND id = $2 FOR KEY SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(run_id)
    .fetch_optional(conn)
    .await?
    .map(|_| ())
    .ok_or_else(|| DbError::NotFound(format!("workflow_run {run_id}")))
}
