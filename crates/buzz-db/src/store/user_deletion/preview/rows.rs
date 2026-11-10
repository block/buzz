//! Exact side-table keys, paged without mutating the snapshot.

use super::*;

const REACTION_KEY: &str = "jsonb_build_object('event_created_at',event_created_at,
    'event_id',encode(event_id,'hex'),'pubkey',encode(pubkey,'hex'),'emoji',emoji)";

// All SQL fragments are static program text. Subject/anchor/cursor are bound.
async fn rows(
    conn: &mut PgConnection,
    output: &mut Output<'_, impl Write>,
    table: &str,
    select: &str,
    anchor: &str,
    reason: &Value,
    action: &str,
) -> Result<()> {
    let query = format!(
        "SELECT row_key,warning FROM ({select}) selected
        WHERE ($3::jsonb IS NULL OR row_key>$3) ORDER BY row_key LIMIT {PAGE_SIZE}"
    );
    let mut cursor: Option<Value> = None;
    loop {
        let page: Vec<(Value, Option<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(query.clone()))
            .bind(output.community)
            .bind(anchor)
            .bind(&cursor)
            .fetch_all(&mut *conn)
            .await?;
        if page.is_empty() {
            break;
        }
        for (key, warning) in page {
            cursor = Some(key.clone());
            output.add(table, key, action, reason, warning)?;
        }
    }
    Ok(())
}

pub(super) async fn event_rows(
    conn: &mut PgConnection,
    event: &Event,
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    let id = hex::encode(&event.id);
    runs(conn, &id, false, output).await?;
    let reason = json!({"type": "event_record", "event_id": id});
    rows(conn, output, "reactions", &format!("SELECT {REACTION_KEY} AS row_key,NULL::text AS warning
        FROM reactions WHERE community_id=$1 AND (event_id=decode($2,'hex') OR reaction_event_id=decode($2,'hex'))"),
        &id, &reason, "would_delete").await?;
    for (table, key, warning) in [
        (
            "event_mentions",
            "jsonb_build_object('pubkey_hex',pubkey_hex,'event_id',encode(event_id,'hex'))",
            "NULL::text",
        ),
        (
            "push_match_queue",
            "jsonb_build_object('event_id',encode(event_id,'hex'))",
            "CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END",
        ),
        (
            "push_wake_outbox",
            "jsonb_build_object('id',id)",
            "CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END",
        ),
        (
            "operator_listener_outbox",
            "jsonb_build_object('id',id)",
            "CASE WHEN lease_until>transaction_timestamp() THEN 'delivery worker is active' END",
        ),
        (
            "delivery_log",
            "jsonb_build_object('delivered_at',delivered_at,'id',id)",
            "NULL::text",
        ),
    ] {
        rows(
            conn,
            output,
            table,
            &format!(
                "SELECT {key} AS row_key,{warning} AS warning
            FROM {table} WHERE community_id=$1 AND event_id=decode($2,'hex')"
            ),
            &id,
            &reason,
            "would_delete",
        )
        .await?;
    }
    Ok(())
}

async fn runs(
    conn: &mut PgConnection,
    anchor: &str,
    by_author: bool,
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    let predicate = if by_author {
        "lower(trigger_context->>'author')=$2"
    } else {
        "(trigger_event_id=decode($2,'hex') OR lower(trigger_context->>'message_id')=$2)"
    };
    let query = format!(
        "SELECT id,status::text FROM workflow_runs WHERE community_id=$1 AND {predicate}
        AND ($3::uuid IS NULL OR id>$3) ORDER BY id LIMIT {PAGE_SIZE}"
    );
    let mut cursor: Option<Uuid> = None;
    loop {
        let page: Vec<(Uuid, String)> = sqlx::query_as(sqlx::AssertSqlSafe(query.clone()))
            .bind(output.community)
            .bind(anchor)
            .bind(cursor)
            .fetch_all(&mut *conn)
            .await?;
        if page.is_empty() {
            break;
        }
        for (run, status) in page {
            cursor = Some(run);
            if output.runs.contains(&run) {
                continue;
            }
            let reason = json!({"type": if by_author {"saved_trigger_author"} else {"trigger_event"}, "source": anchor, "run_id": run});
            rows(conn, output, "workflow_approvals", "SELECT jsonb_build_object('token_hash',encode(token,'hex')) AS row_key,
                NULL::text AS warning FROM workflow_approvals WHERE community_id=$1 AND run_id=$2::uuid",
                &run.to_string(), &reason, "would_delete").await?;
            rows(conn, output, "scheduled_workflow_fires", "SELECT jsonb_build_object('workflow_id',workflow_id,'scheduled_for',scheduled_for) AS row_key,
                NULL::text AS warning FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_run_id=$2::uuid",
                &run.to_string(), &reason, "would_clear_workflow_run_id").await?;
            output.add(
                "workflow_runs",
                json!({"id": run}),
                "would_delete",
                &reason,
                (!matches!(status.as_str(), "completed" | "failed" | "cancelled")).then(|| {
                    format!("workflow run is {status}; must stop before any of its cleanup")
                }),
            )?;
            output.runs.insert(run);
        }
    }
    Ok(())
}

pub(super) async fn orphan_sweeps(
    conn: &mut PgConnection,
    subject: &str,
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    rows(
        conn,
        output,
        "reactions",
        &format!(
            "SELECT {REACTION_KEY} AS row_key,NULL::text AS warning
        FROM reactions WHERE community_id=$1 AND pubkey=decode($2,'hex')"
        ),
        subject,
        &json!({"type": "reaction_author", "pubkey": subject}),
        "would_delete",
    )
    .await?;
    runs(conn, subject, true, output).await
}
