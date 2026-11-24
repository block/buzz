//! Authorized, cursor-paginated Git decision receipts.
use crate::{Db, DbError, Result};
use buzz_core::{
    governance::{DocumentId, RepoCoordinate},
    CommunityId,
};
use serde::Serialize;
use sqlx::Row;

/// One recorded decision; detail also preserves the stage of early refusals.
#[derive(Debug, Serialize)]
pub struct Receipt {
    /// Community audit sequence, used as the pagination cursor.
    pub seq: i64,
    /// git_landed or git_refused.
    pub action: String,
    /// Principal whose operation was evaluated.
    pub actor: Option<String>,
    /// Decision inputs and result as recorded at acceptance.
    pub detail: serde_json::Value,
    /// Audit append time.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A bounded page, newest first.
#[derive(Debug, Serialize)]
pub struct Page {
    /// Recorded decisions in descending sequence order.
    pub receipts: Vec<Receipt>,
    /// Exclusive sequence cursor for the next page, if another row exists.
    pub next_before: Option<i64>,
}

impl Db {
    /// Read receipts only under current repository authority. The caller must
    /// not infer repository existence from a denial.
    // Keep the repository identity, authorization context and page bounds
    // explicit at this database boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn git_receipts(
        &self,
        community: CommunityId,
        repo: &RepoCoordinate,
        actor: &[u8],
        grant: Option<&buzz_core::governance::GitGrant>,
        before: Option<i64>,
        limit: u32,
        open: bool,
    ) -> Result<Page> {
        if !(1..=200).contains(&limit) || before.is_some_and(|n| n <= 0) {
            return Err(DbError::InvalidData("invalid receipt page".into()));
        }
        let mut tx = self.begin_event_write_transaction(community).await?;
        crate::git_land::hold(&mut tx, repo)
            .await?
            .ok_or_else(|| DbError::AccessDenied("restricted: repository read denied".into()))?;
        crate::governance::authorize_read(
            &mut tx,
            &DocumentId::Rules(repo.clone()),
            actor,
            grant,
            open,
        )
        .await?;
        let rows = sqlx::query("SELECT seq,action,encode(actor_pubkey,'hex') AS actor,detail,created_at FROM audit_log WHERE community_id=$1 AND object_id=$2 AND action IN ('git_landed','git_refused') AND ($3::bigint IS NULL OR seq<$3) ORDER BY seq DESC LIMIT $4")
            .bind(community.as_uuid()).bind(repo.to_string()).bind(before).bind(i64::from(limit)+1).fetch_all(tx.conn()).await?;
        let more = rows.len() > limit as usize;
        let receipts = rows
            .into_iter()
            .take(limit as usize)
            .map(|row| {
                Ok(Receipt {
                    seq: row.try_get("seq")?,
                    action: row.try_get("action")?,
                    actor: row.try_get("actor")?,
                    detail: row.try_get("detail")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_before = if more {
            receipts.last().map(|row| row.seq)
        } else {
            None
        };
        // A binding resolver may initialize a legacy pin in this transaction.
        tx.commit().await?;
        Ok(Page {
            receipts,
            next_before,
        })
    }
}
