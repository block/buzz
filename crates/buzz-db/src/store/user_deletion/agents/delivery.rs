use super::*;

pub(super) async fn purge(tx: &mut AdmittedTx, subject: &[u8], limit: u16) -> Result<bool> {
    let community = tx.community();
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
         SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=$2 AND lease_until>clock_timestamp()
         UNION ALL SELECT 1 FROM operator_listener_outbox WHERE community_id=$1
          AND (target_pubkey=$2 OR listener_pubkey=$2) AND lease_until>clock_timestamp())",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .fetch_one(tx.conn())
    .await?;
    if active {
        return Err(safety("agent delivery workers must drain before deletion"));
    }
    for query in [
        "DELETE FROM push_wake_outbox WHERE (community_id,id) IN
         (SELECT community_id,id FROM push_wake_outbox WHERE community_id=$1 AND author=$2 ORDER BY id LIMIT $3)",
        "DELETE FROM operator_listener_outbox WHERE community_id=$1 AND id IN
         (SELECT id FROM operator_listener_outbox WHERE community_id=$1 AND (target_pubkey=$2 OR listener_pubkey=$2)
          ORDER BY id LIMIT $3)",
        "DELETE FROM push_leases WHERE (community_id,author,installation_id) IN
         (SELECT community_id,author,installation_id FROM push_leases WHERE community_id=$1 AND author=$2
          ORDER BY installation_id LIMIT $3)",
    ] {
        if sqlx::query(query)
            .bind(community.as_uuid())
            .bind(subject)
            .bind(i64::from(limit))
            .execute(tx.conn())
            .await?
            .rows_affected() != 0
        {
            return Ok(false);
        }
    }
    let subscription: Option<String> = sqlx::query_scalar(
        "SELECT id FROM subscriptions WHERE community_id=$1 AND owner_pubkey=$2 ORDER BY id LIMIT 1 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .fetch_optional(tx.conn())
    .await?;
    if let Some(id) = subscription {
        let removed = sqlx::query(
            "DELETE FROM delivery_log WHERE community_id=$1 AND (delivered_at,id) IN
             (SELECT delivered_at,id FROM delivery_log WHERE community_id=$1 AND subscription_id=$2
              ORDER BY delivered_at,id LIMIT $3)",
        )
        .bind(community.as_uuid())
        .bind(&id)
        .bind(i64::from(limit))
        .execute(tx.conn())
        .await?
        .rows_affected();
        if removed == 0 {
            sqlx::query(
                "DELETE FROM subscriptions WHERE community_id=$1 AND id=$2 AND owner_pubkey=$3",
            )
            .bind(community.as_uuid())
            .bind(id)
            .bind(subject)
            .execute(tx.conn())
            .await?;
        }
        return Ok(false);
    }
    // Deployment-global listener registrations are retained for finalization,
    // because the same agent key may legitimately participate in other tenants.
    messages::purge_event_kinds(tx, subject, &[30350], limit).await
}
