use super::*;

pub(super) async fn community(
    conn: &mut PgConnection,
    options: &AccountBatchOptions,
) -> Result<()> {
    let safe: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM communities WHERE id=$1 AND deletion_state='active' AND deleted_at IS NULL)
         AND NOT EXISTS(SELECT 1 FROM community_serving_write_leases WHERE community_id=$1 AND lease_until>clock_timestamp())
         AND NOT EXISTS(SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 AND role='owner')
         AND NOT EXISTS(SELECT 1 FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex') AND (agent_owner_pubkey IS NOT NULL OR agent_type IS NOT NULL))")
        .bind(options.community).bind(&options.pubkey).fetch_one(conn).await?;
    if !safe {
        return Err(invalid("requires an active surviving community, a non-owner subject, and stopped external serving work"));
    }
    Ok(())
}

// Match normal writers' parent-before-child locking. Never let a parent DELETE
// cascade to children that were not printed and counted in their own batches.
pub(super) async fn parent(
    conn: &mut PgConnection,
    options: &AccountBatchOptions,
    op: &Operation,
) -> Result<()> {
    if op.update.is_some() {
        return Ok(());
    }
    let query = match op.table {
        "join_policy_acceptances" | "relay_members" =>
            "SELECT pubkey FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR UPDATE",
        "personal_read_frontiers" | "personal_read_accounts" =>
            "SELECT actor FROM personal_read_accounts WHERE community_id=$1 AND actor=decode($2,'hex') FOR UPDATE",
        _ => return Ok(()),
    };
    sqlx::query(query)
        .bind(options.community)
        .bind(&options.pubkey)
        .fetch_all(&mut *conn)
        .await?;
    let query = match op.table {
        "relay_members" => "SELECT EXISTS(SELECT 1 FROM join_policy_acceptances WHERE community_id=$1 AND pubkey=$2)",
        "personal_read_accounts" => "SELECT EXISTS(SELECT 1 FROM personal_read_frontiers WHERE community_id=$1 AND actor=decode($2,'hex'))",
        _ => return Ok(()),
    };
    let children: bool = sqlx::query_scalar(query)
        .bind(options.community)
        .bind(&options.pubkey)
        .fetch_one(conn)
        .await?;
    if children {
        return Err(invalid(
            "parent still has children; current batch rolled back",
        ));
    }
    Ok(())
}
