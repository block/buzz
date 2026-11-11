//! Additional ownership selectors share the event planner's exact-key deduplication.
use super::*;
use rows::rows;

pub(super) async fn plan(
    conn: &mut PgConnection,
    subject: &str,
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    let reason = json!({"type": "owned_resource", "pubkey": subject});
    // Schedule deletion must win over a pointer clear, even when a schedule
    // references a run from another selected workflow.
    rows(conn, output, "scheduled_workflow_fires", "SELECT jsonb_build_object('workflow_id',workflow_id,'scheduled_for',scheduled_for) AS row_key,
        NULL::text AS warning FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id IN
        (SELECT id FROM workflows WHERE community_id=$1 AND owner_pubkey=decode($2,'hex'))",
        subject, &reason, "would_delete").await?;
    let mut cursor: Option<Uuid> = None;
    loop {
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM workflows WHERE community_id=$1 AND owner_pubkey=decode($2,'hex')
            AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT 100",
        )
        .bind(output.community)
        .bind(subject)
        .bind(cursor)
        .fetch_all(&mut *conn)
        .await?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            cursor = Some(id);
            rows::runs(conn, &id.to_string(), "workflow", output).await?;
            rows(conn, output, "workflow_approvals", "SELECT jsonb_build_object('token_hash',encode(token,'hex')) AS row_key,
                NULL::text AS warning FROM workflow_approvals WHERE community_id=$1 AND workflow_id=$2::uuid",
                &id.to_string(), &reason, "would_delete").await?;
            output.add("workflows", json!({"id":id}), "would_delete", &reason, None)?;
        }
    }
    rows(
        conn,
        output,
        "delivery_log",
        "SELECT jsonb_build_object('delivered_at',delivered_at,'id',id) AS row_key,
        NULL::text AS warning FROM delivery_log WHERE community_id=$1 AND subscription_id IN
        (SELECT id FROM subscriptions WHERE community_id=$1 AND owner_pubkey=decode($2,'hex'))",
        subject,
        &reason,
        "would_delete",
    )
    .await?;
    rows(conn, output, "subscriptions", "SELECT jsonb_build_object('id',id) AS row_key,
        NULL::text AS warning FROM subscriptions WHERE community_id=$1 AND owner_pubkey=decode($2,'hex')",
        subject, &reason, "would_delete").await?;
    rows(conn, output, "push_wake_outbox", "SELECT jsonb_build_object('id',id) AS row_key,
        CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END AS warning
        FROM push_wake_outbox WHERE community_id=$1 AND author=decode($2,'hex')",
        subject, &reason, "would_delete").await?;
    rows(conn, output, "push_leases", "SELECT jsonb_build_object('author',encode(author,'hex'),'installation_id',installation_id) AS row_key,
        NULL::text AS warning FROM push_leases WHERE community_id=$1 AND author=decode($2,'hex')",
        subject, &reason, "would_delete").await?;
    rows(conn, output, "operator_listener_outbox", "SELECT jsonb_build_object('id',id) AS row_key,
        CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END AS warning
        FROM operator_listener_outbox WHERE community_id=$1 AND (target_pubkey=decode($2,'hex') OR listener_pubkey=decode($2,'hex'))",
        subject, &reason, "would_delete").await
}
