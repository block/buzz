//! Execution of an internally generated plan, never an imported JSON manifest.
use super::*;
use crate::entity_deletion::EntityLease;
use sqlx::Connection;
use std::collections::HashMap;

impl UserDeletionStore {
    /// Shared entry point for direct commands and workflow content cleanup.
    ///
    /// Read-only mode prints the plan. Delete mode commits bounded batches;
    /// when a workflow lease and stage ID are supplied, each batch also checks
    /// the existing request ownership and approved scope. Reruns select remaining rows.
    pub async fn run_message_batches(
        &self,
        options: &ContentBatchOptions,
        mode: crate::user_deletion::ExecutionMode,
        stage: Option<(&EntityLease, Uuid)>,
        writer: &mut impl Write,
    ) -> Result<ContentBatchSummary> {
        match mode {
            crate::user_deletion::ExecutionMode::ReadOnly => {
                self.preview_message_batches(options, writer).await
            }
            crate::user_deletion::ExecutionMode::Delete => {
                self.execute_message_batches(options, writer, stage).await
            }
        }
    }

    async fn execute_message_batches(
        &self,
        options: &ContentBatchOptions,
        writer: &mut impl Write,
        stage: Option<(&EntityLease, Uuid)>,
    ) -> Result<ContentBatchSummary> {
        let mut conn = self.pool.acquire().await?.detach();
        sqlx::query("SET statement_timeout='30s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET lock_timeout='3s'")
            .execute(&mut conn)
            .await?;
        sqlx::query("SET TIME ZONE 'UTC'")
            .execute(&mut conn)
            .await?;
        let mut plan = plan(&mut conn, options, &mut std::io::sink(), true).await?;
        plan.start["record"] = json!("delete_start");
        plan.start["read_only"] = json!(false);
        plan.start["execution"] =
            json!("print each batch, commit, then print batch_committed; rerun after interruption");
        line(writer, &plan.start)?;
        if plan.summary.warnings != 0 {
            return Err(invalid("selected records have active-work warnings; nothing deleted; inspect --read-only output"));
        }
        // Validate *all* source payloads before deleting even the first child.
        let mut fingerprints = HashMap::new();
        for batch in &plan.batches {
            for item in items(batch)? {
                let (table, predicate) = selection(item)?;
                let hash = inspect(
                    &mut conn,
                    options.community,
                    table,
                    predicate,
                    &item["key"],
                    true,
                )
                .await?;
                fingerprints.insert(identity(item), hash);
            }
        }
        for batch in &plan.batches {
            execute_owned_batch(&mut conn, options, batch, &fingerprints, writer, stage).await?;
        }
        let remaining = plan_remaining(&mut conn, options).await?;
        if remaining != 0 {
            return Err(invalid(
                "supported rows remain after execution; deletion incomplete",
            ));
        }
        plan.summary.record = "delete_complete";
        // This connection owns its session settings and is never returned to the pool.
        conn.close().await?;
        line(writer, &plan.summary)?;
        Ok(plan.summary)
    }
}

async fn plan_remaining(conn: &mut PgConnection, options: &ContentBatchOptions) -> Result<usize> {
    Ok(plan(conn, options, &mut std::io::sink(), false)
        .await?
        .summary
        .items)
}

fn identity(item: &Value) -> String {
    format!("{}:{}", item["table"], item["key"])
}
fn items(batch: &Value) -> Result<&Vec<Value>> {
    batch["items"]
        .as_array()
        .ok_or_else(|| invalid("invalid internal batch"))
}

// SQL identifiers and key expressions are exclusively from this static list.
// JSON keys are bound parameters, never interpolated SQL or caller input.
fn selection(item: &Value) -> Result<(&'static str, &'static str)> {
    Ok(match item["table"].as_str() {
        Some("events") => ("events", "id=decode($2->>'id','hex') AND created_at=($2->>'created_at')::timestamptz"),
        Some("reactions") => ("reactions", "event_id=decode($2->>'event_id','hex') AND event_created_at=($2->>'event_created_at')::timestamptz AND pubkey=decode($2->>'pubkey','hex') AND emoji=$2->>'emoji'"),
        Some("event_mentions") => ("event_mentions", "event_id=decode($2->>'event_id','hex') AND pubkey_hex=$2->>'pubkey_hex'"),
        Some("push_match_queue") => ("push_match_queue", "event_id=decode($2->>'event_id','hex')"),
        Some("push_wake_outbox") => ("push_wake_outbox", "id=($2->>'id')::uuid"),
        Some("operator_listener_outbox") => ("operator_listener_outbox", "id=($2->>'id')::uuid"),
        Some("delivery_log") => ("delivery_log", "id=($2->>'id')::bigint AND delivered_at=($2->>'delivered_at')::timestamptz"),
        Some("workflow_approvals") => ("workflow_approvals", "token=decode($2->>'token_hash','hex')"),
        Some("scheduled_workflow_fires") => ("scheduled_workflow_fires", "workflow_id=($2->>'workflow_id')::uuid AND scheduled_for=($2->>'scheduled_for')::timestamptz"),
        Some("workflow_runs") => ("workflow_runs", "id=($2->>'id')::uuid"),
        Some("workflows") => ("workflows", "id=($2->>'id')::uuid"),
        Some("subscriptions") => ("subscriptions", "id=$2->>'id'"),
        Some("push_leases") => ("push_leases", "author=decode($2->>'author','hex') AND installation_id=$2->>'installation_id'"),
        _ => return Err(invalid("unknown internal batch table")),
    })
}

async fn execute_owned_batch(
    conn: &mut PgConnection,
    options: &ContentBatchOptions,
    batch: &Value,
    fingerprints: &HashMap<String, String>,
    writer: &mut impl Write,
    stage: Option<(&EntityLease, Uuid)>,
) -> Result<()> {
    let community = options.community;
    let mut tx = conn.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await?;
    if let Some((lease, stage_id)) = stage {
        let request = crate::store::entity_deletion::lock_owned(&mut tx, lease).await?;
        crate::store::entity_deletion::validate_approved_inventory(&mut tx, &request).await?;
        let expected = crate::user_deletion::UserDeletionInventory::read(&request)?
            .inputs(&request.target, Some(community))?;
        let inputs: Option<Value> = sqlx::query_scalar("SELECT metadata_json->'inputs' FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$2 AND stage='purge_content' AND target_community_id=$3 AND is_current AND status='running' FOR UPDATE")
            .bind(stage_id).bind(lease.request_id).bind(community).fetch_optional(&mut *tx).await?;
        if request.target != normalize_user_deletion_pubkey(&options.pubkey)?
            || inputs.as_ref() != Some(&expected)
        {
            return Err(invalid("content stage differs from approved scope"));
        }
    }
    // Output must succeed before any mutation in this transaction.
    line(writer, batch)?;
    for item in items(batch)? {
        let (table, predicate) = selection(item)?;
        let hash = inspect(&mut tx, community, table, predicate, &item["key"], false).await?;
        if fingerprints.get(&identity(item)) != Some(&hash) {
            return Err(invalid(format!(
                "{table} row changed after planning; current batch rolled back"
            )));
        }
        children(&mut tx, community, table, &item["key"]).await?;
        let verb = if item["action"] == "would_clear_workflow_run_id"
            && table == "scheduled_workflow_fires"
        {
            "UPDATE scheduled_workflow_fires SET workflow_run_id=NULL".to_owned()
        } else if item["action"] == "would_delete" {
            format!("DELETE FROM {table}")
        } else {
            return Err(invalid("unsupported internally generated action"));
        };
        let sql = format!("{verb} WHERE community_id=$1 AND {predicate}");
        let count = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(community)
            .bind(&item["key"])
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if count != 1 {
            return Err(invalid("planned row missing; current batch rolled back"));
        }
    }
    repair_batch(&mut tx, community, batch).await?;
    if let Some((lease, _)) = stage {
        crate::store::entity_deletion::lock_owned(&mut tx, lease).await?;
    }
    tx.commit().await?;
    line(
        writer,
        &json!({"record":"batch_committed", "number":batch["number"], "items":items(batch)?.len()}),
    )?;
    Ok(())
}

async fn inspect(
    conn: &mut PgConnection,
    community: Uuid,
    table: &str,
    predicate: &str,
    key: &Value,
    preflight: bool,
) -> Result<String> {
    if table == "workflow_approvals" {
        let inconsistent: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workflow_approvals a JOIN workflow_runs r ON r.community_id=a.community_id AND r.id=a.run_id WHERE a.community_id=$1 AND a.token=decode($2->>'token_hash','hex') AND a.workflow_id<>r.workflow_id)").bind(community).bind(key).fetch_one(&mut *conn).await?;
        if inconsistent {
            return Err(invalid(
                "approval and run workflow disagree; reconcile before deletion",
            ));
        }
    }
    let active = match table {
        "workflow_runs" => "status NOT IN ('completed','failed','cancelled')",
        "push_match_queue" => "COALESCE(lease_until>clock_timestamp(),false)",
        "push_wake_outbox" | "operator_listener_outbox" => {
            "state='sending' OR COALESCE(lease_until>clock_timestamp(),false)"
        }
        _ => "false",
    };
    let lock = if preflight { "" } else { " FOR UPDATE" };
    let query = format!("SELECT md5(to_jsonb(t)::text),({active}) FROM {table} t WHERE community_id=$1 AND {predicate}{lock}");
    let (hash, active): (String, bool) = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .bind(community)
        .bind(key)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| invalid(format!("selected {table} row is missing")))?;
    if active {
        return Err(invalid(format!(
            "selected {table} has active work; nothing further deleted"
        )));
    }
    Ok(hash)
}

// No unchecked FK cascade is allowed to do extra work outside the printed
// batch. New children make the current transaction fail rather than disappear.
async fn children(
    conn: &mut PgConnection,
    community: Uuid,
    table: &str,
    key: &Value,
) -> Result<()> {
    let query = match table {
        "workflows" => "SELECT EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_id=($2->>'id')::uuid)",
        "workflow_runs" => "SELECT EXISTS(SELECT 1 FROM workflow_approvals WHERE community_id=$1 AND run_id=($2->>'id')::uuid)
            OR EXISTS(SELECT 1 FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_run_id=($2->>'id')::uuid)",
        "subscriptions" => "SELECT EXISTS(SELECT 1 FROM delivery_log WHERE community_id=$1 AND subscription_id=$2->>'id')",
        "push_leases" => "SELECT EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND author=decode($2->>'author','hex') AND installation_id=$2->>'installation_id')",
        "events" => r#"SELECT EXISTS(SELECT 1 FROM reactions WHERE community_id=$1 AND
    (event_id=decode($2->>'id','hex') OR reaction_event_id=decode($2->>'id','hex')))
OR EXISTS(SELECT 1 FROM event_mentions WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM push_match_queue WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM operator_listener_outbox WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM delivery_log WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND
    (trigger_event_id=decode($2->>'id','hex') OR lower(trigger_context->>'message_id')=$2->>'id'))
OR EXISTS(SELECT 1 FROM events e CROSS JOIN LATERAL (
    SELECT decode(t->>1,'hex') AS id FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
    WHERE t->>0='e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
    ORDER BY CASE WHEN e.kind IN (40003,45002) THEN n ELSE -n END LIMIT 1) target
    WHERE e.community_id=$1 AND e.kind IN (7,40003,45002) AND target.id=decode($2->>'id','hex')
      AND e.channel_id IS NOT DISTINCT FROM
        (SELECT channel_id FROM events WHERE community_id=$1 AND id=decode($2->>'id','hex')
          AND created_at=($2->>'created_at')::timestamptz))"#,
        _ => return Ok(()),
    };
    let remains: bool = sqlx::query_scalar(query)
        .bind(community)
        .bind(key)
        .fetch_one(conn)
        .await?;
    if remains {
        return Err(invalid(format!(
            "{table} still has dependent rows; current batch rolled back"
        )));
    }
    Ok(())
}

async fn repair_batch(conn: &mut PgConnection, community: Uuid, batch: &Value) -> Result<()> {
    let repairs = batch["thread_repairs"]
        .as_array()
        .ok_or_else(|| invalid("missing internal repairs"))?;
    let mut seen = HashSet::new();
    for repair in repairs {
        let mut roots = repair["stored_threads"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // A surviving reply can name a root whose own metadata never existed.
        roots.push(json!({"channel_id":repair["channel_id"], "root_event_id":repair["event_id"]}));
        for root in roots {
            if !seen.insert(root.to_string()) {
                continue;
            }
            let channel: Option<Uuid> = serde_json::from_value(root["channel_id"].clone())?;
            let id = root["root_event_id"]
                .as_str()
                .ok_or_else(|| invalid("missing repair root"))?;
            let id = hex::decode(id).map_err(|_| invalid("invalid repair root"))?;
            repair_thread(conn, community, channel, &id).await?;
        }
    }
    Ok(())
}

async fn repair_thread(
    conn: &mut PgConnection,
    community: Uuid,
    channel: Option<Uuid>,
    root: &[u8],
) -> Result<()> {
    // Preserve only the ancestry needed by retained events. The missing event
    // rows carry no content, author, tags or signature; metadata keeps replies
    // reachable without manufacturing a signed replacement event.
    sqlx::query(
        "WITH RECURSIVE needed AS (
          SELECT tm.* FROM thread_metadata tm
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND (tm.event_id=$3 OR tm.root_event_id=$3)
            AND EXISTS (SELECT 1 FROM events e WHERE e.community_id=tm.community_id
              AND e.id=tm.event_id AND e.created_at=tm.event_created_at)
          UNION
          SELECT p.* FROM thread_metadata p JOIN needed n ON p.community_id=n.community_id
            AND p.channel_id IS NOT DISTINCT FROM n.channel_id AND (p.event_id=n.parent_event_id OR p.event_id=n.root_event_id)
        ) DELETE FROM thread_metadata tm WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid
          AND (tm.event_id=$3 OR tm.root_event_id=$3)
          AND NOT EXISTS (SELECT 1 FROM needed n WHERE n.event_id=tm.event_id AND n.event_created_at=tm.event_created_at)",
    ).bind(community).bind(channel).bind(root).execute(&mut *conn).await?;
    // Recompute, never subtract old counts. Already-soft-deleted messages do
    // not count; received_at matches the server-arrival activity clock.
    // Like live writes, last_reply_at tracks direct replies, not descendants.
    sqlx::query(
        "WITH visible AS MATERIALIZED (
          SELECT tm.parent_event_id,tm.root_event_id,e.received_at FROM thread_metadata tm
          JOIN events e ON e.community_id=tm.community_id AND e.id=tm.event_id AND e.created_at=tm.event_created_at
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND tm.root_event_id=$3 AND e.deleted_at IS NULL
        ), direct AS (
          SELECT parent_event_id,count(*)::int AS n,max(received_at) AS last_at FROM visible GROUP BY parent_event_id
        ), descendants AS (
          SELECT root_event_id,count(*)::int AS n FROM visible GROUP BY root_event_id
        ), counts AS (
          SELECT tm.event_id,tm.event_created_at,COALESCE(d.n,0) AS replies,COALESCE(r.n,0) AS descendants,
            d.last_at
          FROM thread_metadata tm LEFT JOIN direct d ON d.parent_event_id=tm.event_id
          LEFT JOIN descendants r ON r.root_event_id=tm.event_id
          WHERE tm.community_id=$1 AND tm.channel_id IS NOT DISTINCT FROM $2::uuid AND (tm.event_id=$3 OR tm.root_event_id=$3)
        ) UPDATE thread_metadata tm SET reply_count=c.replies,descendant_count=c.descendants,last_reply_at=c.last_at
          FROM counts c WHERE tm.community_id=$1 AND tm.event_id=c.event_id AND tm.event_created_at=c.event_created_at",
    ).bind(community).bind(channel).bind(root).execute(&mut *conn).await?;
    Ok(())
}
