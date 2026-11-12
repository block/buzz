//! Exact side-table keys, paged without mutating the snapshot.

use super::*;

const REACTION_KEY: &str = "jsonb_build_object('event_created_at',event_created_at,
    'event_id',encode(event_id,'hex'),'pubkey',encode(pubkey,'hex'),'emoji',emoji)";

// All SQL fragments are static program text. Subject/anchor/cursor are bound.
pub(super) async fn rows(
    conn: &mut PgConnection,
    output: &mut Output<'_, impl Write>,
    table: &str,
    select: &str,
    anchors: &[Vec<u8>],
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
            .bind(anchors)
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
    let reason = json!({"type": "event_record", "event_id": id});
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
    ] {
        rows(
            conn,
            output,
            table,
            &format!(
                "SELECT {key} AS row_key,{warning} AS warning
            FROM {table} WHERE community_id=$1 AND event_id=ANY($2::bytea[])"
            ),
            std::slice::from_ref(&event.id),
            &reason,
            "would_delete",
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn authored_reactions(
    conn: &mut PgConnection,
    targets: &[Vec<u8>],
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    rows(
        conn,
        output,
        "reactions",
        &format!(
            "SELECT {REACTION_KEY} AS row_key,NULL::text AS warning
        FROM reactions WHERE community_id=$1 AND pubkey=ANY($2::bytea[])"
        ),
        targets,
        &json!({"type": "reaction_author"}),
        "would_delete",
    )
    .await
}
