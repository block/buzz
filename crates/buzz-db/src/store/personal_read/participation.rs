//! Conversation membership from the existing thread store, bounded by time.
use super::model::ELIGIBLE_KINDS;
use crate::Result;
use buzz_core::CommunityId;
use sqlx::{Acquire, PgConnection, Row};
use std::collections::HashMap;
use uuid::Uuid;

// Bound multiplicative work independently of the unread evidence window.
const MAX_PARENTS: usize = 1024;

/// Whether each `(channel, parent)` is one of the actor's conversations: the
/// actor wrote the parent or has a reply to it. Only live eligible messages
/// qualify. An absent key is undecided: past the target cap, or the statement
/// deadline expired. Membership is independent of unread retention and read
/// frontiers.
pub(super) async fn resolve(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8],
    targets: &[(Uuid, Vec<u8>)],
) -> Result<HashMap<(Uuid, Vec<u8>), bool>> {
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    let targets = select_targets(targets);
    let channels: Vec<_> = targets.iter().map(|(channel, _)| *channel).collect();
    let parents: Vec<_> = targets.iter().map(|(_, parent)| parent.clone()).collect();
    // Optional inference must not abort authoritative unread/frontier reads.
    // A nested transaction is a savepoint; rollback also restores the caller's
    // statement timeout. No state is written in this read-only inference.
    let mut budget = conn.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = '500ms'")
        .execute(&mut *budget)
        .await?;
    sqlx::query("SET LOCAL jit = off")
        .execute(&mut *budget)
        .await?;
    // Exact: a row cap would lose a real member behind a busy parent's other
    // replies. LATERAL ... LIMIT 1 keeps the work on the replies under the
    // requested parents; a plain EXISTS may be hashed over the whole tenant.
    let result = sqlx::query(
        "SELECT t.channel_id,t.parent_id,(own.hit OR replied.hit) IS TRUE AS member
         FROM unnest($3::uuid[],$4::bytea[]) t(channel_id,parent_id)
         LEFT JOIN LATERAL (
            SELECT e.pubkey=$2 AND e.deleted_at IS NULL AND e.kind=ANY($5) AS hit
            FROM events e WHERE e.community_id=$1 AND e.channel_id=t.channel_id AND e.id=t.parent_id
            ORDER BY e.created_at DESC LIMIT 1
         ) own ON true
         LEFT JOIN LATERAL (
            SELECT true AS hit FROM thread_metadata tm JOIN events e ON e.community_id=$1
                AND e.channel_id=t.channel_id AND e.created_at=tm.event_created_at AND e.id=tm.event_id
            WHERE tm.community_id=$1 AND tm.channel_id=t.channel_id AND tm.parent_event_id=t.parent_id
                AND e.pubkey=$2 AND e.deleted_at IS NULL AND e.kind=ANY($5)
                AND own.hit IS NOT TRUE
            LIMIT 1
         ) replied ON true",
    )
    .bind(community.as_uuid())
    .bind(actor)
    .bind(channels)
    .bind(parents)
    .bind(ELIGIBLE_KINDS.as_slice())
    .fetch_all(&mut *budget)
    .await;
    budget.rollback().await?;
    let rows = match result {
        Ok(rows) => rows,
        Err(sqlx::Error::Database(error))
            if matches!(error.code().as_deref(), Some("57014" | "55P03")) =>
        {
            return Ok(HashMap::new());
        }
        Err(error) => return Err(error.into()),
    };
    rows.into_iter()
        .map(|row| {
            Ok((
                (row.try_get("channel_id")?, row.try_get("parent_id")?),
                row.try_get("member")?,
            ))
        })
        .collect()
}

// Keep selection independent of the optional SQL deadline: a timeout must not
// hide a regression in the cardinality bound.
fn select_targets(targets: &[(Uuid, Vec<u8>)]) -> Vec<(Uuid, Vec<u8>)> {
    let mut targets = targets.to_vec();
    targets.sort_unstable();
    targets.dedup();
    targets.truncate(MAX_PARENTS);
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_selection_caps_unique_parents_independently_of_sql_timeout() {
        // Literal contract boundaries deliberately do not derive from MAX_PARENTS.
        for count in [0_u32, 1, 1023, 1024, 1025] {
            let unique: Vec<_> = (0..count)
                .map(|i| (Uuid::nil(), i.to_be_bytes().to_vec()))
                .collect();
            let input: Vec<_> = unique
                .iter()
                .rev()
                .chain(unique.iter().rev())
                .cloned()
                .collect();
            let selected = select_targets(&input);
            assert_eq!(selected, unique[..unique.len().min(1024)], "count {count}");
        }
    }

    #[test]
    fn target_selection_keeps_channel_identity() {
        let first = (Uuid::from_u128(1), vec![7; 32]);
        let second = (Uuid::from_u128(2), vec![7; 32]);
        assert_eq!(
            select_targets(&[second.clone(), first.clone(), second.clone()]),
            vec![first, second]
        );
    }
}
