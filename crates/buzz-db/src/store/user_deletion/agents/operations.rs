use super::*;

// Profiles, emoji sets, owner-authored configuration and shared artifacts are
// deliberately excluded. These remain inputs to media/identity finalization.
pub(super) const PRIVATE_KINDS: &[i32] = &[
    3, 10000, 10001, 10002, 10003, 30000, 30003, 30315, 30078, 30174, 44200, 30300,
];
pub(super) const JOB_KINDS: &[i32] = &[43001, 43002, 43003, 43004, 43005, 43006];

pub(super) async fn private_state(tx: &mut AdmittedTx, subject: &[u8], limit: u16) -> Result<bool> {
    if !messages::purge_event_kinds(tx, subject, PRIVATE_KINDS, limit).await? {
        return Ok(false);
    }
    let community = tx.community();
    let removed = sqlx::query(
        "DELETE FROM personal_read_frontiers WHERE (community_id,actor,channel_id,root_id) IN
         (SELECT community_id,actor,channel_id,root_id FROM personal_read_frontiers
          WHERE community_id=$1 AND actor=$2 ORDER BY channel_id,root_id LIMIT $3)",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .bind(i64::from(limit))
    .execute(tx.conn())
    .await?
    .rows_affected();
    if removed != 0 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM personal_read_accounts WHERE community_id=$1 AND actor=$2")
        .bind(community.as_uuid())
        .bind(subject)
        .execute(tx.conn())
        .await?;
    Ok(true)
}

pub(super) async fn verify(
    tx: &mut AdmittedTx,
    subject: &[u8],
    subject_hex: &str,
    relay: &[u8; 32],
) -> Result<()> {
    if messages::has_remaining(tx, subject, subject_hex, relay).await? {
        return Err(safety("agent verification found remaining message data"));
    }
    let community = tx.community();
    let kinds = PRIVATE_KINDS
        .iter()
        .chain(JOB_KINDS)
        .copied()
        .chain([30620, 30350])
        .collect::<Vec<_>>();
    let remaining: bool = sqlx::query_scalar(
        "SELECT EXISTS (
         SELECT 1 FROM events WHERE community_id=$1 AND pubkey=$2 AND kind=ANY($3)
         UNION ALL SELECT 1 FROM personal_read_accounts WHERE community_id=$1 AND actor=$2
         UNION ALL SELECT 1 FROM personal_read_frontiers WHERE community_id=$1 AND actor=$2
         UNION ALL SELECT 1 FROM workflows WHERE community_id=$1 AND owner_pubkey=$2
         UNION ALL SELECT 1 FROM subscriptions WHERE community_id=$1 AND owner_pubkey=$2
         UNION ALL SELECT 1 FROM push_leases WHERE community_id=$1 AND author=$2
         UNION ALL SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=$2
         UNION ALL SELECT 1 FROM operator_listener_outbox WHERE community_id=$1
           AND (target_pubkey=$2 OR listener_pubkey=$2))",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .bind(kinds)
    .fetch_one(tx.conn())
    .await?;
    if remaining {
        return Err(safety(
            "agent verification found remaining supported database content",
        ));
    }
    Ok(())
}
