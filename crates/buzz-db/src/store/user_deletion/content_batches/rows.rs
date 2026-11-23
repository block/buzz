//! Exact side-table keys, paged without mutating the snapshot.

use super::*;

const REACTION_KEY: &str = "jsonb_build_object('event_created_at',event_created_at,
    'event_id',encode(event_id,'hex'),'pubkey',encode(pubkey,'hex'),'emoji',emoji)";

#[derive(Clone, Copy)]
pub(super) enum Selection<'a> {
    Targets(&'a [Vec<u8>]),
    Events {
        ids: &'a [Vec<u8>],
        authors: &'a [Vec<u8>],
    },
}

// All SQL fragments are static program text. Subject/anchor/cursor are bound.
pub(super) async fn rows(
    conn: &mut PgConnection,
    output: &mut Output<'_, impl Write>,
    table: &'static str,
    select: &str,
    selection: Selection<'_>,
    reason: &Value,
    action: &str,
) -> Result<()> {
    let query = format!(
        "SELECT row_key,warning,subject_keys FROM ({select}) selected
        WHERE ($3::jsonb IS NULL OR row_key>$3) ORDER BY row_key LIMIT {PAGE_SIZE}"
    );
    let mut cursor: Option<Value> = None;
    loop {
        let query = sqlx::query_as(sqlx::AssertSqlSafe(query.clone())).bind(output.community);
        let query = match selection {
            Selection::Targets(targets) => query.bind(targets).bind(&cursor),
            Selection::Events { ids, authors } => query.bind(ids).bind(&cursor).bind(authors),
        };
        let page: Vec<(Value, Option<String>, Vec<Vec<u8>>)> = query.fetch_all(&mut *conn).await?;
        if page.is_empty() {
            break;
        }
        for (key, warning, subject_keys) in page {
            cursor = Some(key.clone());
            output.add(table, key, action, reason, warning, &subject_keys)?;
        }
    }
    Ok(())
}

pub(super) async fn event_rows(
    conn: &mut PgConnection,
    events: &[Event],
    output: &mut Output<'_, impl Write>,
) -> Result<()> {
    let ids: Vec<Vec<u8>> = events.iter().map(|e| e.id.clone()).collect();
    let authors: Vec<Vec<u8>> = events.iter().map(|e| e.attributed_pubkey.clone()).collect();
    let (source, subject_keys, selection) = if output.attribution.is_some() {
        (
            "JOIN (SELECT source_id,array_agg(author) AS subject_keys
                FROM unnest($2::bytea[],$4::bytea[]) evidence(source_id,author)
                GROUP BY source_id) source ON source.source_id=t.event_id",
            "source.subject_keys",
            Selection::Events {
                ids: &ids,
                authors: &authors,
            },
        )
    } else {
        ("", "ARRAY[]::bytea[]", Selection::Targets(&ids))
    };
    let reason = json!({"type": "event_records"});
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
                "SELECT {key} AS row_key,{warning} AS warning,{subject_keys} AS subject_keys
            FROM {table} t {source} WHERE community_id=$1 AND event_id=ANY($2::bytea[])"
            ),
            selection,
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
            "SELECT {REACTION_KEY} AS row_key,NULL::text AS warning,ARRAY[pubkey] AS subject_keys
        FROM reactions WHERE community_id=$1 AND pubkey=ANY($2::bytea[])"
        ),
        Selection::Targets(targets),
        &json!({"type": "reaction_author"}),
        "would_delete",
    )
    .await
}
