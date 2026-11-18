//! Additional ownership selectors share the event planner's exact-key deduplication.
use super::*;
use rows::rows;

pub(super) async fn plan(
    conn: &mut PgConnection,
    targets: &[Vec<u8>],
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    let reason = json!({"type": "owned_resource"});
    // Workflow state can contain another user's approvals or work. Leave it
    // intact in this initial ownership-only content pass.
    rows(
        conn,
        output,
        "delivery_log",
        "SELECT jsonb_build_object('delivered_at',delivered_at,'id',id) AS row_key,
        NULL::text AS warning FROM delivery_log WHERE community_id=$1 AND subscription_id IN
        (SELECT id FROM subscriptions WHERE community_id=$1 AND owner_pubkey=ANY($2::bytea[]))",
        targets,
        &reason,
        "would_delete",
    )
    .await?;
    rows(conn, output, "subscriptions", "SELECT jsonb_build_object('id',id) AS row_key,
        NULL::text AS warning FROM subscriptions WHERE community_id=$1 AND owner_pubkey=ANY($2::bytea[])",
        targets, &reason, "would_delete").await?;
    rows(conn, output, "push_wake_outbox", "SELECT jsonb_build_object('id',id) AS row_key,
        CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END AS warning
        FROM push_wake_outbox WHERE community_id=$1 AND author=ANY($2::bytea[])",
        targets, &reason, "would_delete").await?;
    rows(conn, output, "push_leases", "SELECT jsonb_build_object('author',encode(author,'hex'),'installation_id',installation_id) AS row_key,
        NULL::text AS warning FROM push_leases WHERE community_id=$1 AND author=ANY($2::bytea[])",
        targets, &reason, "would_delete").await?;
    rows(conn, output, "operator_listener_outbox", "SELECT jsonb_build_object('id',id) AS row_key,
        CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END AS warning
        FROM operator_listener_outbox WHERE community_id=$1 AND (target_pubkey=ANY($2::bytea[]) OR listener_pubkey=ANY($2::bytea[]))",
        targets, &reason, "would_delete").await
}
