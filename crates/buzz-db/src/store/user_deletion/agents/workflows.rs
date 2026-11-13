use super::*;

pub(super) async fn purge(tx: &mut AdmittedTx, subject: &[u8], limit: u16) -> Result<bool> {
    let community = tx.community();
    let workflow: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, definition::text ~* 'https?://|/media/' FROM workflows
         WHERE community_id=$1 AND owner_pubkey=$2 ORDER BY id LIMIT 1 FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(subject)
    .fetch_optional(tx.conn())
    .await?;
    if let Some((id, has_links)) = workflow {
        if has_links {
            return Err(safety("agent workflow has link references; preserve media/resource inputs before deletion"));
        }
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM workflow_runs WHERE community_id=$1 AND workflow_id=$2
             AND status NOT IN ('completed','failed','cancelled'))",
        )
        .bind(community.as_uuid())
        .bind(id)
        .fetch_one(tx.conn())
        .await?;
        if active {
            return Err(safety(
                "agent-owned workflow runs must stop before deletion",
            ));
        }
        // Child-first explicit deletes. Each call commits at most one bounded
        // child batch; the workflow row remains the recovery anchor until empty.
        for query in [
            "DELETE FROM scheduled_workflow_fires WHERE (community_id,workflow_id,scheduled_for) IN
             (SELECT community_id,workflow_id,scheduled_for FROM scheduled_workflow_fires
              WHERE community_id=$1 AND workflow_id=$2 ORDER BY scheduled_for LIMIT $3)",
            "DELETE FROM workflow_approvals WHERE (community_id,token) IN
             (SELECT community_id,token FROM workflow_approvals WHERE community_id=$1
              AND (workflow_id=$2 OR run_id IN (SELECT id FROM workflow_runs WHERE community_id=$1 AND workflow_id=$2))
              ORDER BY token LIMIT $3)",
            "DELETE FROM workflow_runs WHERE (community_id,id) IN
             (SELECT community_id,id FROM workflow_runs WHERE community_id=$1 AND workflow_id=$2 ORDER BY id LIMIT $3)",
        ] {
            if sqlx::query(query)
                .bind(community.as_uuid())
                .bind(id)
                .bind(i64::from(limit))
                .execute(tx.conn())
                .await?
                .rows_affected() != 0
            {
                return Ok(false);
            }
        }
        sqlx::query("DELETE FROM workflows WHERE community_id=$1 AND id=$2 AND owner_pubkey=$3")
            .bind(community.as_uuid())
            .bind(id)
            .bind(subject)
            .execute(tx.conn())
            .await?;
        return Ok(false);
    }
    messages::purge_event_kinds(tx, subject, &[30620], limit).await
}
