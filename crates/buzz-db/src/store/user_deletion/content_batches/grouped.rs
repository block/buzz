//! Session worklist and set-based validation/mutations. No caller-supplied SQL.
use super::*;
use crate::AdmittedTx;
use execute::{items, selection};

fn tables(batch: &Value) -> Result<Vec<&str>> {
    let mut tables = Vec::new();
    for item in items(batch)? {
        let (table, _) = selection(item)?;
        if !tables.contains(&table) {
            tables.push(table);
        }
    }
    // A batch can straddle selection pages, with an event from the previous
    // page preceding the next page's side rows. Always remove dependants first.
    tables.sort_by_key(|table| match *table {
        "subscriptions" | "push_leases" => 1,
        "events" => 2,
        _ => 0,
    });
    Ok(tables)
}

fn predicate(table: &str) -> Result<String> {
    Ok(selection(&json!({"table":table}))?
        .1
        .replace("$2", "w.row_key"))
}

fn active(table: &str) -> &'static str {
    match table {
        "push_match_queue" => "COALESCE(t.lease_until>clock_timestamp(),false)",
        "push_wake_outbox" | "operator_listener_outbox" => {
            "t.state='sending' OR COALESCE(t.lease_until>clock_timestamp(),false)"
        }
        _ => "false",
    }
}

pub(super) async fn prepare(conn: &mut PgConnection, community: Uuid, plan: &Plan) -> Result<()> {
    sqlx::query(
        "CREATE TEMP TABLE content_batch_work (
        table_name text NOT NULL, row_key jsonb NOT NULL, batch bigint NOT NULL,
        action text NOT NULL, fingerprint text, PRIMARY KEY(table_name,row_key))",
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query("CREATE INDEX ON content_batch_work(batch,table_name)")
        .execute(&mut *conn)
        .await?;
    let mut all_tables = HashSet::new();
    for batch in &plan.batches {
        sqlx::query(
            "INSERT INTO content_batch_work(table_name,row_key,batch,action)
            SELECT i->>'table',i->'key',$1,i->>'action' FROM jsonb_array_elements($2) i",
        )
        .bind(batch["number"].as_i64())
        .bind(&batch["items"])
        .execute(&mut *conn)
        .await?;
        all_tables.extend(tables(batch)?);
    }
    for table in all_tables {
        let pred = predicate(table)?;
        let active = active(table);
        let query = format!(
            "UPDATE content_batch_work w SET fingerprint=md5(to_jsonb(t)::text)
            FROM {table} t WHERE w.table_name=$2 AND t.community_id=$1 AND {pred}
              AND NOT({active})"
        );
        sqlx::query(sqlx::AssertSqlSafe(query))
            .bind(community)
            .bind(table)
            .execute(&mut *conn)
            .await?;
    }
    let missing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM content_batch_work WHERE fingerprint IS NULL)",
    )
    .fetch_one(&mut *conn)
    .await?;
    if missing {
        return Err(invalid(
            "selected rows changed or have active work; nothing deleted",
        ));
    }
    Ok(())
}

pub(super) async fn execute(tx: &mut AdmittedTx, batch: &Value) -> Result<()> {
    let community = *tx.community().as_uuid();
    let number = batch["number"]
        .as_i64()
        .ok_or_else(|| invalid("missing batch number"))?;
    for table in tables(batch)? {
        let pred = predicate(table)?;
        let active = active(table);
        let query = format!(
            "SELECT w.fingerprint=md5(to_jsonb(t)::text) AND NOT({active})
            FROM {table} t JOIN content_batch_work w ON w.table_name=$2 AND {pred}
            WHERE t.community_id=$1 AND w.batch=$3 FOR UPDATE OF t"
        );
        let matches: Vec<bool> = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(community)
            .bind(table)
            .bind(number)
            .fetch_all(tx.conn())
            .await?;
        let expected = items(batch)?.iter().filter(|i| i["table"] == table).count();
        if matches.len() != expected || matches.iter().any(|m| !m) {
            return Err(invalid(format!(
                "{table} rows changed or have active work; current batch rolled back"
            )));
        }
        children(tx.conn(), community, number, table).await?;
        if table == "events" {
            erase_events(tx, number).await?;
        } else {
            let query = format!(
                "DELETE FROM {table} t USING content_batch_work w
                WHERE t.community_id=$1 AND w.table_name=$2 AND w.batch=$3 AND {pred}
                  AND w.action='would_delete'"
            );
            let affected = sqlx::query(sqlx::AssertSqlSafe(query))
                .bind(community)
                .bind(table)
                .bind(number)
                .execute(tx.conn())
                .await?
                .rows_affected();
            if affected as usize != expected {
                return Err(invalid(
                    "planned rows missing or unsupported action; current batch rolled back",
                ));
            }
        }
    }
    Ok(())
}

async fn children(
    conn: &mut PgConnection,
    community: Uuid,
    number: i64,
    table: &str,
) -> Result<()> {
    let dependent = match table {
        "subscriptions" => "EXISTS(SELECT 1 FROM delivery_log WHERE community_id=$1 AND subscription_id=w.row_key->>'id')",
        "push_leases" => "EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=decode(w.row_key->>'author','hex') AND installation_id=w.row_key->>'installation_id')",
        "events" => "EXISTS(SELECT 1 FROM event_mentions WHERE community_id=$1 AND event_id=decode(w.row_key->>'id','hex'))
            OR EXISTS(SELECT 1 FROM push_match_queue WHERE community_id=$1 AND event_id=decode(w.row_key->>'id','hex'))
            OR EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND event_id=decode(w.row_key->>'id','hex'))
            OR EXISTS(SELECT 1 FROM operator_listener_outbox WHERE community_id=$1 AND event_id=decode(w.row_key->>'id','hex'))",
        _ => return Ok(()),
    };
    let query = format!(
        "SELECT EXISTS(SELECT 1 FROM content_batch_work w
        WHERE w.table_name=$2 AND w.batch=$3 AND ({dependent}))"
    );
    let remains: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .bind(community)
        .bind(table)
        .bind(number)
        .fetch_one(conn)
        .await?;
    if remains {
        return Err(invalid(
            "new dependent records appeared; current batch rolled back; rerun to plan them",
        ));
    }
    Ok(())
}

async fn erase_events(tx: &mut AdmittedTx, number: i64) -> Result<()> {
    let community = *tx.community().as_uuid();
    // Only a currently unreferenced row may be physically removed. Also check
    // selected rows in later batches; a concurrent reply must never lose a link.
    let referenced: bool = sqlx::query_scalar(include_str!("recheck.sql"))
        .bind(community)
        .bind(number)
        .fetch_one(tx.conn())
        .await?;
    if referenced {
        return Err(invalid("new surviving references appeared; current batch rolled back; rerun to classify delete versus scrub"));
    }
    // Keep counters consistent in every committed prefix, including interruption.
    sqlx::query(include_str!("account.sql"))
        .bind(community)
        .bind(number)
        .execute(tx.conn())
        .await?;
    sqlx::query("UPDATE events e SET pubkey=''::bytea,content='',sig=''::bytea,
        tags=COALESCE((SELECT jsonb_agg(CASE WHEN tag->>3 IN ('root','reply')
            THEN jsonb_build_array('e',lower(tag->>1),'',tag->>3)
            ELSE jsonb_build_array('e',lower(tag->>1)) END)
            FROM jsonb_array_elements(e.tags) tag WHERE tag->>0='e'
              AND tag->>1 ~ '^[0-9a-fA-F]{64}$'),'[]'::jsonb),
        d_tag=NULL,not_before=NULL,delivered_at=NULL,
        deleted_at=COALESCE(deleted_at,clock_timestamp())
        FROM content_batch_work w WHERE w.table_name='events' AND w.batch=$2
        AND w.action='would_scrub' AND e.community_id=$1
        AND e.id=decode(w.row_key->>'id','hex') AND e.created_at=(w.row_key->>'created_at')::timestamptz")
        .bind(community).bind(number).execute(tx.conn()).await?;
    sqlx::query(
        "DELETE FROM thread_metadata tm USING content_batch_work w
        WHERE w.table_name='events' AND w.batch=$2 AND w.action='would_delete'
          AND tm.community_id=$1 AND tm.event_id=decode(w.row_key->>'id','hex')
          AND tm.event_created_at=(w.row_key->>'created_at')::timestamptz",
    )
    .bind(community)
    .bind(number)
    .execute(tx.conn())
    .await?;
    sqlx::query(
        "DELETE FROM events e USING content_batch_work w
        WHERE w.table_name='events' AND w.batch=$2 AND w.action='would_delete'
          AND e.community_id=$1 AND e.id=decode(w.row_key->>'id','hex')
          AND e.created_at=(w.row_key->>'created_at')::timestamptz",
    )
    .bind(community)
    .bind(number)
    .execute(tx.conn())
    .await?;
    Ok(())
}

pub(super) async fn verify_remaining(
    conn: &mut PgConnection,
    options: &ContentBatchOptions,
    start: &Value,
) -> Result<()> {
    let targets: Vec<Vec<u8>> = start["target_pubkeys"]
        .as_array()
        .ok_or_else(|| invalid("missing internal targets"))?
        .iter()
        .map(|k| {
            hex::decode(k.as_str().unwrap_or_default())
                .map_err(|_| invalid("invalid internal target"))
        })
        .collect::<Result<_>>()?;
    let relay = start["trusted_relay_pubkey"].as_str();
    let query = format!("WITH authored AS ({}) SELECT EXISTS(SELECT 1 FROM authored WHERE kind=ANY($4))
        OR EXISTS(SELECT 1 FROM reactions WHERE community_id=$1 AND pubkey=ANY($2))
        OR EXISTS(SELECT 1 FROM subscriptions WHERE community_id=$1 AND owner_pubkey=ANY($2))
        OR EXISTS(SELECT 1 FROM push_leases WHERE community_id=$1 AND author=ANY($2))
        OR EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=ANY($2))
        OR EXISTS(SELECT 1 FROM operator_listener_outbox WHERE community_id=$1 AND (target_pubkey=ANY($2) OR listener_pubkey=ANY($2)))",include_str!("authored.sql"));
    let remains: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .bind(options.community)
        .bind(targets)
        .bind(relay)
        .bind(KINDS)
        .fetch_one(conn)
        .await?;
    if remains {
        return Err(invalid(
            "supported rows remain after execution; deletion incomplete; rerun",
        ));
    }
    Ok(())
}
